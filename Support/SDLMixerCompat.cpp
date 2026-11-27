/**
 *  SDLMixerCompat.cpp
 *  ONScripter-RU
 *
 *  SDL2_mixer compatibility surface backed by SDL3_mixer.
 *
 *  Consult LICENSE file for licensing terms and copyright holders.
 */

#include "Support/SDLMixerCompat.hpp"

#if defined(ONS_USE_SDL3)

#include "Support/FileDefs.hpp"

#include <algorithm>
#include <cctype>
#include <cstring>
#include <limits>
#include <string>
#include <vector>

struct Mix_Music {
	MIX_Audio *audio{nullptr};
	Mix_MusicType type{MUS_NONE};
};

namespace {

struct ChannelState {
	MIX_Track *track{nullptr};
	Mix_Chunk *chunk{nullptr};
	SDL_AudioStream *stream{nullptr};
	float volume{static_cast<float>(MIX_MAX_VOLUME)};
	int index{-1};
	Mix_EffectFunc_t effect{nullptr};
	Mix_EffectDone_t effectDone{nullptr};
	void *effectUserdata{nullptr};
};

MIX_Mixer *mixer{nullptr};
MIX_Track *musicTrack{nullptr};
Mix_Music *currentMusic{nullptr};
SDL_AudioSpec mixerSpec{MIX_DEFAULT_FORMAT, MIX_DEFAULT_CHANNELS, MIX_DEFAULT_FREQUENCY};
std::vector<ChannelState> channels;
void(SDLCALL *channelFinishedCallback)(int channel){nullptr};
void(SDLCALL *musicFinishedCallback)(){nullptr};
float musicVolume{static_cast<float>(MIX_MAX_VOLUME)};
std::string musicCommand;

float clampVolumeFloat(float volume) {
	if (volume < 0.0f)
		return volume;
	return std::clamp(volume, 0.0f, static_cast<float>(MIX_MAX_VOLUME));
}

float volumeToGain(float volume) {
	constexpr float invMaxVolume = 1.0f / static_cast<float>(MIX_MAX_VOLUME);
	return std::max(0.0f, clampVolumeFloat(volume)) * invMaxVolume;
}

int volumeFloatToInt(float volume) {
	return static_cast<int>(clampVolumeFloat(volume) + 0.5f);
}

void setTrackVolume(MIX_Track *track, float volume) {
	MIX_SetTrackGain(track, volumeToGain(volume));
}

ChannelState *getChannel(int channel) {
	if (channel < 0 || channel >= static_cast<int>(channels.size()))
		return nullptr;
	return &channels[channel];
}

void clearChannelInput(ChannelState &channel) {
	if (channel.track) {
		MIX_StopTrack(channel.track, 0);
		MIX_SetTrackAudio(channel.track, nullptr);
		MIX_SetTrackAudioStream(channel.track, nullptr);
	}
	if (channel.stream) {
		SDL_DestroyAudioStream(channel.stream);
		channel.stream = nullptr;
	}
	channel.chunk = nullptr;
}

void SDLCALL trackStopped(void * /*userdata*/, MIX_Track *track) {
	if (track == musicTrack) {
		currentMusic = nullptr;
		if (musicFinishedCallback)
			musicFinishedCallback();
		return;
	}

	for (auto &channel : channels) {
		if (channel.track == track) {
			channel.chunk = nullptr;
			if (channelFinishedCallback)
				channelFinishedCallback(channel.index);
			return;
		}
	}
}

void SDLCALL streamGetCallback(void *userdata, SDL_AudioStream *stream, int additionalAmount, int /*totalAmount*/) {
	ChannelState *channel = static_cast<ChannelState *>(userdata);
	if (!channel || !channel->chunk || !channel->chunk->abuf || channel->chunk->alen == 0)
		return;

	int remaining = std::max(additionalAmount, static_cast<int>(channel->chunk->alen));
	while (remaining > 0) {
		if (channel->effect)
			channel->effect(channel->index, channel->chunk->abuf, static_cast<int>(channel->chunk->alen), channel->effectUserdata);

		if (!SDL_PutAudioStreamData(stream, channel->chunk->abuf, static_cast<int>(channel->chunk->alen)))
			return;
		remaining -= static_cast<int>(channel->chunk->alen);
	}
}

Mix_MusicType detectMusicType(SDL_RWops *src) {
	if (!src)
		return MUS_NONE;

	const Sint64 pos = SDL_TellIO(src);
	Uint8 header[16]{};
	const size_t read = SDL_ReadIO(src, header, sizeof(header));
	if (pos >= 0)
		SDL_SeekIO(src, pos, SDL_IO_SEEK_SET);

	if (read >= 12 && std::memcmp(header, "RIFF", 4) == 0 && std::memcmp(header + 8, "WAVE", 4) == 0)
		return MUS_WAV;
	if (read >= 4 && std::memcmp(header, "OggS", 4) == 0)
		return MUS_OGG;
	if (read >= 3 && std::memcmp(header, "ID3", 3) == 0)
		return MUS_MP3;
	if (read >= 2 && header[0] == 0xff && (header[1] & 0xe0) == 0xe0)
		return MUS_MP3;

	return MUS_NONE;
}

Mix_MusicType detectMusicType(const char *file) {
	if (!file)
		return MUS_NONE;

	std::string path(file);
	std::transform(path.begin(), path.end(), path.begin(), [](unsigned char c) {
		return static_cast<char>(std::tolower(c));
	});

	if (path.size() >= 4 && path.compare(path.size() - 4, 4, ".wav") == 0)
		return MUS_WAV;
	if (path.size() >= 4 && path.compare(path.size() - 4, 4, ".ogg") == 0)
		return MUS_OGG;
	if (path.size() >= 4 && path.compare(path.size() - 4, 4, ".mp3") == 0)
		return MUS_MP3;
	return MUS_NONE;
}

SDL_PropertiesID makePlayProperties(int loops, bool streamInput) {
	if (loops == 0 && !streamInput)
		return 0;

	SDL_PropertiesID props = SDL_CreateProperties();
	if (!props)
		return 0;

	SDL_SetNumberProperty(props, MIX_PROP_PLAY_LOOPS_NUMBER, loops);
	if (streamInput)
		SDL_SetBooleanProperty(props, MIX_PROP_PLAY_HALT_WHEN_EXHAUSTED_BOOLEAN, false);
	return props;
}

bool ensureAudioReady() {
	if (mixer)
		return true;
	SDL_SetError("SDL3_mixer compatibility mixer is not open");
	return false;
}

} // namespace

const char *Mix_GetError() {
	return SDL_GetError();
}

int Mix_OpenAudio(int frequency, SDL_AudioFormat format, int channelCount, int chunksize) {
	return Mix_OpenAudioDevice(frequency, format, channelCount, chunksize, nullptr, 0);
}

int Mix_OpenAudioDevice(int frequency, SDL_AudioFormat format, int channelCount, int chunksize, const char * /*device*/, int /*allowed_changes*/) {
	Mix_CloseAudio();

	if (!MIX_Init())
		return -1;

	SDL_AudioSpec requested{};
	requested.freq     = frequency;
	requested.format   = format;
	requested.channels = channelCount;

	SDL_SetHint(SDL_HINT_AUDIO_DEVICE_SAMPLE_FRAMES, chunksize > 0 ? std::to_string(chunksize).c_str() : nullptr);
	mixer = MIX_CreateMixerDevice(SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK, &requested);
	if (!mixer) {
		MIX_Quit();
		return -1;
	}

	MIX_GetMixerFormat(mixer, &mixerSpec);
	SDL_AudioSpec deviceSpec{};
	int deviceFrames{};
	if (SDL_GetAudioDeviceFormat(SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK, &deviceSpec, &deviceFrames)) {
		const char *frameHint = SDL_GetHint(SDL_HINT_AUDIO_DEVICE_SAMPLE_FRAMES);
		sendToLog(LogLevel::Info, "Audio device: %s, %d Hz %s, %d channels, %d frames per callback "
		                          "(requested %d Hz %s, %s frames)\n",
		          SDL_GetCurrentAudioDriver(), deviceSpec.freq, SDL_GetAudioFormatName(deviceSpec.format),
		          deviceSpec.channels, deviceFrames, frequency, SDL_GetAudioFormatName(format), frameHint ? frameHint : "auto");
	}
	musicTrack = MIX_CreateTrack(mixer);
	if (!musicTrack) {
		Mix_CloseAudio();
		return -1;
	}
	MIX_SetTrackStoppedCallback(musicTrack, trackStopped, nullptr);
	setTrackVolume(musicTrack, musicVolume);
	return 0;
}

void Mix_CloseAudio() {
	for (auto &channel : channels) {
		clearChannelInput(channel);
		if (channel.track) {
			MIX_DestroyTrack(channel.track);
			channel.track = nullptr;
		}
	}
	channels.clear();

	if (musicTrack) {
		MIX_StopTrack(musicTrack, 0);
		MIX_DestroyTrack(musicTrack);
		musicTrack = nullptr;
	}
	currentMusic = nullptr;

	if (mixer) {
		MIX_DestroyMixer(mixer);
		mixer = nullptr;
		MIX_Quit();
	}
}

int Mix_QuerySpec(int *frequency, SDL_AudioFormat *format, int *channelCount) {
	if (!mixer)
		return 0;
	if (frequency)
		*frequency = mixerSpec.freq;
	if (format)
		*format = mixerSpec.format;
	if (channelCount)
		*channelCount = mixerSpec.channels;
	return 1;
}

int Mix_AllocateChannels(int numchans) {
	if (!ensureAudioReady())
		return 0;

	numchans = std::max(0, numchans);
	if (numchans < static_cast<int>(channels.size())) {
		for (int i = numchans; i < static_cast<int>(channels.size()); ++i) {
			clearChannelInput(channels[i]);
			MIX_DestroyTrack(channels[i].track);
		}
		channels.resize(numchans);
	} else {
		const int oldSize = static_cast<int>(channels.size());
		channels.resize(numchans);
		for (int i = oldSize; i < numchans; ++i) {
			channels[i].index = i;
			channels[i].track = MIX_CreateTrack(mixer);
			if (channels[i].track) {
				setTrackVolume(channels[i].track, channels[i].volume);
				MIX_SetTrackStoppedCallback(channels[i].track, trackStopped, nullptr);
			}
		}
	}

	return static_cast<int>(channels.size());
}

void Mix_ChannelFinished(void(SDLCALL *callback)(int channel)) {
	channelFinishedCallback = callback;
}

int Mix_Volume(int channel, int volume) {
	return Mix_VolumeFloat(channel, static_cast<float>(volume));
}

int Mix_VolumeFloat(int channel, float volume) {
	if (channel == -1) {
		int previous = 0;
		const bool setVolume = volume >= 0.0f;
		const float clamped  = setVolume ? clampVolumeFloat(volume) : volume;
		for (auto &state : channels) {
			previous += volumeFloatToInt(state.volume);
			if (setVolume && state.volume != clamped) {
				state.volume = clamped;
				if (state.track)
					setTrackVolume(state.track, state.volume);
			}
		}
		return channels.empty() ? 0 : previous / static_cast<int>(channels.size());
	}

	ChannelState *state = getChannel(channel);
	if (!state)
		return 0;
	const int previous = volumeFloatToInt(state->volume);
	if (volume >= 0.0f) {
		const float clamped = clampVolumeFloat(volume);
		if (state->volume != clamped) {
			state->volume = clamped;
			if (state->track)
				setTrackVolume(state->track, state->volume);
		}
	}
	return previous;
}

int Mix_VolumeMusic(int volume) {
	return Mix_VolumeMusicFloat(static_cast<float>(volume));
}

int Mix_VolumeMusicFloat(float volume) {
	const int previous = volumeFloatToInt(musicVolume);
	if (volume >= 0.0f) {
		const float clamped = clampVolumeFloat(volume);
		if (musicVolume != clamped) {
			musicVolume = clamped;
			if (musicTrack)
				setTrackVolume(musicTrack, musicVolume);
		}
	}
	return previous;
}

Mix_Chunk *Mix_LoadWAV_RW(SDL_RWops *src, int freesrc, size_t maxDecodedBytes) {
	if (!ensureAudioReady())
		return nullptr;

	MIX_AudioDecoder *decoder = MIX_CreateAudioDecoder_IO(src, freesrc != 0, 0);
	if (!decoder)
		return nullptr;

	std::vector<Uint8> buffer(64 * 1024);
	Uint8 *decoded         = nullptr;
	size_t decodedSize     = 0;
	size_t decodedCapacity = 0;
	for (;;) {
		const int bytes = MIX_DecodeAudio(decoder, buffer.data(), static_cast<int>(buffer.size()), &mixerSpec);
		if (bytes < 0) {
			SDL_free(decoded);
			MIX_DestroyAudioDecoder(decoder);
			return nullptr;
		}
		if (bytes == 0)
			break;
		const size_t required = decodedSize + static_cast<size_t>(bytes);
		if (required > static_cast<size_t>(std::numeric_limits<Uint32>::max()) ||
		    (maxDecodedBytes && required > maxDecodedBytes)) {
			SDL_free(decoded);
			MIX_DestroyAudioDecoder(decoder);
			return nullptr;
		}
		if (required > decodedCapacity) {
			size_t newCapacity = std::max(required, decodedCapacity ? decodedCapacity * 2 : buffer.size());
			newCapacity        = std::min(newCapacity, static_cast<size_t>(std::numeric_limits<Uint32>::max()));
			if (maxDecodedBytes)
				newCapacity = std::min(newCapacity, maxDecodedBytes);
			void *newDecoded   = SDL_realloc(decoded, newCapacity);
			if (!newDecoded) {
				SDL_free(decoded);
				MIX_DestroyAudioDecoder(decoder);
				return nullptr;
			}
			decoded         = static_cast<Uint8 *>(newDecoded);
			decodedCapacity = newCapacity;
		}
		std::memcpy(decoded + decodedSize, buffer.data(), static_cast<size_t>(bytes));
		decodedSize = required;
	}
	MIX_DestroyAudioDecoder(decoder);

	if (decodedSize == 0) {
		SDL_free(decoded);
		return nullptr;
	}

	Mix_Chunk *chunk = new Mix_Chunk();
	chunk->alen      = static_cast<Uint32>(decodedSize);
	// Decoding grows geometrically. Retain only the finished samples, and
	// account for the old capacity if the allocator cannot shrink in place.
	if (decodedCapacity > decodedSize) {
		if (void *compact = SDL_realloc(decoded, decodedSize)) {
			decoded = static_cast<Uint8 *>(compact);
			decodedCapacity = decodedSize;
		}
	}
	chunk->capacityBytes = decodedCapacity;
	chunk->abuf      = decoded;
	chunk->allocated = 1;
	chunk->audio     = MIX_LoadRawAudioNoCopy(mixer, chunk->abuf, chunk->alen, &mixerSpec, false);
	if (!chunk->audio) {
		SDL_free(chunk->abuf);
		delete chunk;
		return nullptr;
	}
	return chunk;
}

Mix_Chunk *Mix_QuickLoad_RAW(Uint8 *mem, Uint32 len) {
	Mix_Chunk *chunk = new Mix_Chunk();
	chunk->abuf      = mem;
	chunk->alen      = len;
	chunk->quickRaw  = true;
	return chunk;
}

void Mix_FreeChunk(Mix_Chunk *chunk) {
	if (!chunk)
		return;
	if (chunk->audio)
		MIX_DestroyAudio(chunk->audio);
	if (chunk->allocated && chunk->abuf)
		SDL_free(chunk->abuf);
	delete chunk;
}

int Mix_RegisterEffect(int chan, Mix_EffectFunc_t f, Mix_EffectDone_t d, void *arg) {
	ChannelState *state = getChannel(chan);
	if (!state)
		return 0;
	state->effect         = f;
	state->effectDone     = d;
	state->effectUserdata = arg;
	return 1;
}

int Mix_PlayChannel(int channel, Mix_Chunk *chunk, int loops) {
	if (!ensureAudioReady() || !chunk)
		return -1;

	if (channel < 0) {
		for (int i = 0; i < static_cast<int>(channels.size()); ++i) {
			if (!Mix_Playing(i)) {
				channel = i;
				break;
			}
		}
	}

	ChannelState *state = getChannel(channel);
	if (!state || !state->track)
		return -1;

	clearChannelInput(*state);
	state->chunk = chunk;

	bool streamInput = false;
	if (chunk->quickRaw) {
		if (state->effect) {
			state->stream = SDL_CreateAudioStream(&mixerSpec, &mixerSpec);
			if (!state->stream)
				return -1;
			SDL_SetAudioStreamGetCallback(state->stream, streamGetCallback, state);
			if (!MIX_SetTrackAudioStream(state->track, state->stream))
				return -1;
			streamInput = true;
		} else {
			if (!chunk->audio)
				chunk->audio = MIX_LoadRawAudioNoCopy(mixer, chunk->abuf, chunk->alen, &mixerSpec, false);
			if (!chunk->audio)
				return -1;
			if (!MIX_SetTrackAudio(state->track, chunk->audio))
				return -1;
		}
	} else if (!MIX_SetTrackAudio(state->track, chunk->audio)) {
		return -1;
	}

	setTrackVolume(state->track, state->volume);
	SDL_PropertiesID props = makePlayProperties(loops, streamInput);
	const bool ok          = MIX_PlayTrack(state->track, props);
	if (props)
		SDL_DestroyProperties(props);
	if (ok)
		setTrackVolume(state->track, state->volume);
	return ok ? channel : -1;
}

int Mix_Playing(int channel) {
	if (channel == -1) {
		int count = 0;
		for (int i = 0; i < static_cast<int>(channels.size()); ++i)
			count += Mix_Playing(i) ? 1 : 0;
		return count;
	}

	ChannelState *state = getChannel(channel);
	return (state && state->track && MIX_TrackPlaying(state->track)) ? 1 : 0;
}

void Mix_Pause(int channel) {
	if (channel == -1) {
		for (auto &state : channels)
			if (state.track)
				MIX_PauseTrack(state.track);
		return;
	}

	ChannelState *state = getChannel(channel);
	if (state && state->track)
		MIX_PauseTrack(state->track);
}

int Mix_Paused(int channel) {
	if (channel == -1) {
		int count = 0;
		for (int i = 0; i < static_cast<int>(channels.size()); ++i)
			count += Mix_Paused(i) ? 1 : 0;
		return count;
	}

	ChannelState *state = getChannel(channel);
	return (state && state->track && MIX_TrackPaused(state->track)) ? 1 : 0;
}

int Mix_HaltChannel(int channel) {
	if (channel == -1) {
		for (int i = 0; i < static_cast<int>(channels.size()); ++i)
			Mix_HaltChannel(i);
		return 0;
	}

	ChannelState *state = getChannel(channel);
	if (!state || !state->track)
		return -1;
	clearChannelInput(*state);
	if (state->effectDone)
		state->effectDone(channel, state->effectUserdata);
	return 0;
}

Mix_Music *Mix_LoadMUS(const char *file) {
	if (!ensureAudioReady())
		return nullptr;

	MIX_Audio *audio = MIX_LoadAudio(mixer, file, false);
	if (!audio)
		return nullptr;

	Mix_Music *music = new Mix_Music();
	music->audio     = audio;
	music->type      = detectMusicType(file);
	return music;
}

Mix_Music *Mix_LoadMUS_RW(SDL_RWops *src, int freesrc) {
	if (!ensureAudioReady())
		return nullptr;

	const Mix_MusicType type = detectMusicType(src);
	MIX_Audio *audio        = MIX_LoadAudio_IO(mixer, src, false, freesrc != 0);
	if (!audio)
		return nullptr;

	Mix_Music *music = new Mix_Music();
	music->audio     = audio;
	music->type      = type;
	return music;
}

void Mix_FreeMusic(Mix_Music *music) {
	if (!music)
		return;
	if (currentMusic == music)
		currentMusic = nullptr;
	if (music->audio)
		MIX_DestroyAudio(music->audio);
	delete music;
}

Mix_MusicType Mix_GetMusicType(const Mix_Music *music) {
	return music ? music->type : MUS_NONE;
}

void Mix_HookMusicFinished(void(SDLCALL *callback)()) {
	musicFinishedCallback = callback;
}

int Mix_PlayMusic(Mix_Music *music, int loops) {
	if (!ensureAudioReady() || !musicTrack || !music || !music->audio)
		return -1;

	MIX_StopTrack(musicTrack, 0);
	if (!MIX_SetTrackAudio(musicTrack, music->audio))
		return -1;

	setTrackVolume(musicTrack, musicVolume);
	SDL_PropertiesID props = makePlayProperties(loops, false);
	const bool ok          = MIX_PlayTrack(musicTrack, props);
	if (props)
		SDL_DestroyProperties(props);
	if (ok) {
		setTrackVolume(musicTrack, musicVolume);
		currentMusic = music;
	}
	return ok ? 0 : -1;
}

int Mix_PlayingMusic() {
	return (musicTrack && MIX_TrackPlaying(musicTrack)) ? 1 : 0;
}

int Mix_HaltMusic() {
	if (!musicTrack)
		return -1;
	MIX_StopTrack(musicTrack, 0);
	currentMusic = nullptr;
	return 0;
}

int Mix_SetMusicCMD(const char *command) {
	musicCommand = command ? command : "";
	return 0;
}

void *Mix_GetMusicHookData() {
	return nullptr;
}

#endif
