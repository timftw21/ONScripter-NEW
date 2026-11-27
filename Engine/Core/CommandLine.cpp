#include "Engine/Core/CommandLine.hpp"

#include <array>

namespace CommandLine {

bool optionRequiresValue(std::string_view option) {
	static constexpr std::array options{
	    "--audiodriver", "--audiobuffer", "--audio-buffer", "--audio-freq", "--audioformat", "--renderer-blacklist",
#ifdef DROID
	    "--audio-low-latency",
#endif
	    "--prefer-renderer", "--registry", "--dll", "-r", "--root", "--tmp-root",
	    "-s", "--save", "--window-width", "--gameid", "--nsa-offset", "--automode-time",
	    "--voicedelay-time", "--voicewait-time", "--final-voicedelay-time", "--game-script",
	    "--game_script", "--force-fps", "--discord-app-id", "--sdl3-benchmark-iterations",
	    "--sdl3-benchmark-width", "--sdl3-benchmark-height", "--sdl3-benchmark-output",
	    "--musicbox-benchmark-output", "--texture-upload", "--render-self", "--texlimit",
	    "--chunklimit", "--mouse-scrollmul", "--touch-scrollmul", "--ramlimit", "--hwdecoder",
	    "--hwconvert", "--breakup", "--glassbreak", "--reduce-motion", "--font-overrides",
	    "--font-multiplier", "--lang-dir", "--font-dir", "--dialogue-style", "--cursor",
	    "--pad-map", "--prefer-rumble", "--system-offset-x", "--system-offset-y",
	    "-NSDocumentRevisionsDebugMode"};
	for (const std::string_view candidate : options)
		if (option == candidate)
			return true;
	return environmentOptionName(option).has_value();
}

std::optional<std::string> environmentOptionName(std::string_view option) {
	static constexpr std::string_view Prefix = "--env[";
	if (!option.starts_with(Prefix) || option.size() <= Prefix.size() + 1 || option.back() != ']')
		return std::nullopt;
	const std::string_view name = option.substr(Prefix.size(), option.size() - Prefix.size() - 1);
	auto isAsciiAlpha = [](unsigned char ch) {
		return (ch >= 'A' && ch <= 'Z') || (ch >= 'a' && ch <= 'z');
	};
	if (name.size() > 128 || !(isAsciiAlpha(static_cast<unsigned char>(name.front())) || name.front() == '_'))
		return std::nullopt;
	for (const unsigned char ch : name)
		if (!(isAsciiAlpha(ch) || (ch >= '0' && ch <= '9') || ch == '_'))
			return std::nullopt;
	return std::string(name);
}

bool validate(const std::vector<std::string_view> &arguments, std::string &error) {
	error.clear();
	static constexpr size_t MaximumArgumentLength = 32768;
	for (const std::string_view argument : arguments) {
		if (argument.size() > MaximumArgumentLength) {
			error = "command-line argument is too long";
			return false;
		}
	}
	for (size_t i = 0; i < arguments.size(); ++i) {
		if (arguments[i].starts_with("--env[") && !environmentOptionName(arguments[i])) {
			error = "invalid environment option name";
			return false;
		}
		if (optionRequiresValue(arguments[i]) && i + 1 == arguments.size()) {
			error = "option requires a value: " + std::string(arguments[i]);
			return false;
		}
		if (optionRequiresValue(arguments[i]))
			++i;
	}
	return true;
}

} // namespace CommandLine
