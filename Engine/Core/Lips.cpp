/**
 *  Lips.cpp
 *  ONScripter-RU
 *
 *  Provides lipsync implementation.
 *
 *  Consult LICENSE file for licensing terms and copyright holders.
 */

#include "Engine/Core/ONScripter.hpp"

#include <cstdio>
#include <cstring>
#include <limits>

namespace {
bool lipsTelemetryEnabled() {
	const char *value = onsSDLGetEnv("ONS_SDL3_GPU_TELEMETRY");
	return value && *value && std::strcmp(value, "0") != 0;
}
}

bool ONScripter::LipsAnimationAction::expired() {
	if (ons.skipLipsAction)
		return false; // can't expire if we're told not even to do anything
	// Deal with non-playing channels.

	return !ons.wave_sample[channel] || !Mix_Playing(channel) || Mix_Paused(channel);
}

bool ONScripter::LipsAnimationAction::setCellForCharacters(const std::vector<std::string> &characterNames, int cellNumber) {
	if (characterNames.empty())
		return false;

	const bool collectTelemetry = lipsTelemetryEnabled();
	if (collectTelemetry)
		++ons.lipsTelemetry.targetCellUpdates;

	bool changed{false};
	auto matchesCharacter = [&](const char *lipsName) {
		if (!lipsName)
			return false;
		for (const auto &characterName : characterNames) {
			if (characterName == lipsName)
				return true;
		}
		return false;
	};

	auto setCell = [&](AnimationInfo *ai, bool oldSprite) {
		if (ai->exists && ai->gpu_image && ai->visible && ai->lips_name) {
			if (matchesCharacter(ai->lips_name) && ai->current_cell != cellNumber) {
				ai->setCell(cellNumber);
				ons.dirtySpriteRect(ai->id, ai->type == SPRITE_LSP2, oldSprite);
				changed = true;
				if (collectTelemetry)
					++ons.lipsTelemetry.spriteCellChanges;
			}
		}
	};

	auto scanSprites = [&](AnimationInfo *sprites) {
		if (!sprites)
			return;
		for (int i = 0; i < MAX_SPRITE_NUM; ++i) {
			AnimationInfo *ai = &sprites[i];
			if (!ai->exists && (!ai->old_ai || !ai->old_ai->exists))
				continue;
			setCell(ai, false);
			if (ai->old_ai)
				setCell(ai->old_ai, true);
		}
	};

	scanSprites(ons.sprite_info);
	scanSprites(ons.sprite2_info);

	if (changed) {
		if (collectTelemetry)
			++ons.lipsTelemetry.flushes;
		ons.flush(ons.refreshMode());
	}
	return changed;
}

bool ONScripter::LipsAnimationAction::draw() {
	// do this even if expired on last call. onExpire
	if (channel < 0 || channel >= static_cast<int>(ons.lipsChannels.size()) || !ons.lipsChannels[channel].has())
		return false;
	return setCellForCharacters(ons.lipsChannels[channel].get().characterNames, 0);
}

void ONScripter::LipsAnimationAction::onExpired() {
	ConstantRefreshAction::onExpired();
	if (draw() && lipsTelemetryEnabled())
		++ons.lipsTelemetry.expiredCloses;
}

void ONScripter::LipsAnimationAction::run() {
	const bool collectTelemetry = lipsTelemetryEnabled();
	if (collectTelemetry)
		++ons.lipsTelemetry.runCalls;

	if (ons.skipLipsAction)
		return;

	// Deal with playing channels.
	if (!ons.wave_sample[channel] || !Mix_Playing(channel) || Mix_Paused(channel))
		return;

	if (channel < 0 || channel >= static_cast<int>(ons.lipsChannels.size()) || !ons.lipsChannels[channel].has())
		return;

	int now = SDL_GetTicks();
	Lips &lipdata = ons.lipsChannels[channel].get().lipsData;

	int index = static_cast<int>((0.0 + now - lipdata.speechStart) / MS_PER_CHUNK);
	int cellNumber = (index >= 0 && index < lipdata.seqSize) ? lipdata.seq[index] : 0;
	if (!setCellForCharacters(ons.lipsChannels[channel].get().characterNames, cellNumber) && collectTelemetry)
		++ons.lipsTelemetry.noChangeRuns;
}

double ONScripter::readChunk(int channel, uint32_t no) {
	switch (audio_format.format) {
		case AUDIO_S8:
			return std::abs(*reinterpret_cast<int8_t *>(wave_sample[channel]->chunk->abuf + no));
		case AUDIO_U8:
			return wave_sample[channel]->chunk->abuf[no];
		case AUDIO_S16:
			return std::abs(*reinterpret_cast<int16_t *>(wave_sample[channel]->chunk->abuf + no));
#if !defined(ONS_USE_SDL3)
		case AUDIO_U16:
			return *reinterpret_cast<uint16_t *>(wave_sample[channel]->chunk->abuf + no);
#endif
		case AUDIO_S32:
			return std::abs(*reinterpret_cast<int32_t *>(wave_sample[channel]->chunk->abuf + no));
		case AUDIO_F32:
		default:
			return std::fabs(*reinterpret_cast<float *>(wave_sample[channel]->chunk->abuf + no));
	}
}

void ONScripter::getChunkParams(uint32_t &chunk_size, double &max_value) {
	switch (audio_format.format) {
		case AUDIO_S8:
			chunk_size = sizeof(int8_t) * audio_format.channels;
			max_value  = std::numeric_limits<int8_t>::max();
			break;
		case AUDIO_U8:
			chunk_size = sizeof(uint8_t) * audio_format.channels;
			max_value  = std::numeric_limits<uint8_t>::max();
			break;
		case AUDIO_S16:
			chunk_size = sizeof(int16_t) * audio_format.channels;
			max_value  = std::numeric_limits<int16_t>::max();
			break;
#if !defined(ONS_USE_SDL3)
		case AUDIO_U16:
			chunk_size = sizeof(uint16_t) * audio_format.channels;
			max_value  = std::numeric_limits<uint16_t>::max();
			break;
#endif
		case AUDIO_S32:
			chunk_size = sizeof(int32_t) * audio_format.channels;
			max_value  = std::numeric_limits<int32_t>::max();
			break;
		case AUDIO_F32:
		default:
			chunk_size = sizeof(float) * audio_format.channels;
			max_value  = 1.0;
			break;
	}
}

void ONScripter::loadLips(int channel) {
	const bool collectTelemetry = lipsTelemetryEnabled();
	if (collectTelemetry)
		++lipsTelemetry.loadCalls;

	uint32_t chunk_size;
	double max_value;
	getChunkParams(chunk_size, max_value);

	uint32_t buf_len = wave_sample[channel]->chunk->alen;
	if (buf_len > static_cast<uint64_t>(audio_format.freq) * MAX_SOUND_LENGTH * chunk_size) {
		sendToLog(LogLevel::Error, "The file is too big!\n");
		return;
	}

	uint32_t i{0};
	int vc{0}, phase{0};
	double peak{0};
	Lips &lipdata = lipsChannels[channel].get().lipsData;

	do {
		auto v = readChunk(channel, i);
		if (v > peak)
			peak = v;

		phase += CHUNKS_PER_SECOND;
		i += chunk_size;

		if (phase >= audio_format.freq || i >= buf_len) {
			peak /= max_value;

			lipdata.seq[vc] = 2;
			if (peak < speechLevels[1])
				lipdata.seq[vc] = 1;
			if (peak < speechLevels[0])
				lipdata.seq[vc] = 0;

			if (vc == 0 && lipdata.seq[vc] == 2)
				lipdata.seq[vc] = 1;
			if (vc > 0) {
				if ((lipdata.seq[vc] == 2 && lipdata.seq[vc - 1] == 0) ||
				    (lipdata.seq[vc] == 0 && lipdata.seq[vc - 1] == 2)) {
					lipdata.seq[vc] = 1;
				}
			}
			if (i >= buf_len && lipdata.seq[vc] == 2)
				lipdata.seq[vc] = 1;

			vc++;

			phase -= audio_format.freq;
			peak = 0;
		}
	} while (i < buf_len);

	lipdata.seqSize = vc;
	if (collectTelemetry)
		lipsTelemetry.loadedChunks += vc;
}

void ONScripter::printLipsTelemetry() const {
	if (!lipsTelemetryEnabled() || lipsTelemetryPrinted)
		return;
	if (lipsTelemetry.loadCalls == 0 && lipsTelemetry.runCalls == 0 && lipsTelemetry.spriteCellChanges == 0)
		return;

	lipsTelemetryPrinted = true;
	std::printf("Lips telemetry: load_calls=%llu loaded_chunks=%llu run_calls=%llu "
	            "target_cell_updates=%llu sprite_cell_changes=%llu flushes=%llu "
	            "no_change_runs=%llu expired_closes=%llu\n",
	            static_cast<unsigned long long>(lipsTelemetry.loadCalls),
	            static_cast<unsigned long long>(lipsTelemetry.loadedChunks),
	            static_cast<unsigned long long>(lipsTelemetry.runCalls),
	            static_cast<unsigned long long>(lipsTelemetry.targetCellUpdates),
	            static_cast<unsigned long long>(lipsTelemetry.spriteCellChanges),
	            static_cast<unsigned long long>(lipsTelemetry.flushes),
	            static_cast<unsigned long long>(lipsTelemetry.noChangeRuns),
	            static_cast<unsigned long long>(lipsTelemetry.expiredCloses));
}
