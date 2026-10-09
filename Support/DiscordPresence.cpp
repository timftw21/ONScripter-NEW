/**
 *  DiscordPresence.cpp
 *  ONScripter-RU
 *
 *  Minimal Discord Rich Presence client over local Discord RPC IPC.
 *
 *  Consult LICENSE file for licensing terms and copyright holders.
 */

#include "Support/DiscordPresence.hpp"

#include "Support/FileIO.hpp"

#include <algorithm>
#include <cctype>
#include <chrono>
#include <ctime>
#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>

#ifdef WIN32
#include <windows.h>
#elif defined(LINUX) || defined(MACOSX)
#include <cerrno>
#include <sys/select.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>
#endif

DiscordPresence discordPresence;

#ifdef ONS_DISCORD_PRESENCE_SUPPORTED
namespace {

constexpr uint32_t OpcodeHandshake = 0;
constexpr uint32_t OpcodeFrame     = 1;
constexpr uint32_t OpcodeClose     = 2;
constexpr uint32_t OpcodePing      = 3;
constexpr uint32_t OpcodePong      = 4;
constexpr size_t MaxFramePayload   = 64 * 1024;
constexpr uint32_t ServiceIntervalMs{250};
constexpr uint32_t ReconnectIntervalMs{15000};
constexpr uint32_t RefreshIntervalMs{30000};

uint64_t nowMilliseconds() {
	using Clock = std::chrono::steady_clock;
	return static_cast<uint64_t>(std::chrono::duration_cast<std::chrono::milliseconds>(Clock::now().time_since_epoch()).count());
}

void writeLE32(uint8_t *dst, uint32_t value) {
	dst[0] = static_cast<uint8_t>(value & 0xff);
	dst[1] = static_cast<uint8_t>((value >> 8) & 0xff);
	dst[2] = static_cast<uint8_t>((value >> 16) & 0xff);
	dst[3] = static_cast<uint8_t>((value >> 24) & 0xff);
}

uint32_t readLE32(const uint8_t *src) {
	return static_cast<uint32_t>(src[0]) |
	       (static_cast<uint32_t>(src[1]) << 8) |
	       (static_cast<uint32_t>(src[2]) << 16) |
	       (static_cast<uint32_t>(src[3]) << 24);
}

bool validApplicationId(const std::string &id) {
	return id.size() >= 16 && id.size() <= 32 &&
	       std::all_of(id.begin(), id.end(), [](unsigned char ch) {
		       return std::isdigit(ch) != 0;
	       });
}

std::string jsonEscaped(const std::string &value) {
	std::string out;
	out.reserve(value.size() + 8);
	static constexpr char Hex[] = "0123456789abcdef";

	for (unsigned char ch : value) {
		switch (ch) {
			case '"': out += "\\\""; break;
			case '\\': out += "\\\\"; break;
			case '\b': out += "\\b"; break;
			case '\f': out += "\\f"; break;
			case '\n': out += "\\n"; break;
			case '\r': out += "\\r"; break;
			case '\t': out += "\\t"; break;
			default:
				if (ch < 0x20) {
					out += "\\u00";
					out += Hex[(ch >> 4) & 0xf];
					out += Hex[ch & 0xf];
				} else {
					out += static_cast<char>(ch);
				}
				break;
		}
	}

	return out;
}

uint32_t currentProcessId() {
#ifdef WIN32
	return static_cast<uint32_t>(GetCurrentProcessId());
#elif defined(LINUX) || defined(MACOSX)
	return static_cast<uint32_t>(getpid());
#else
	return 0;
#endif
}

#if defined(LINUX) || defined(MACOSX)
std::vector<std::string> discordIpcSearchRoots() {
	std::vector<std::string> roots;
	for (const char *name : {"XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"}) {
		const char *value = std::getenv(name);
		if (value && value[0] != '\0')
			roots.emplace_back(value);
	}
	roots.emplace_back("/tmp");
	return roots;
}
#endif

} // namespace
#endif

bool DiscordPresence::start(const std::string &id) {
#ifdef ONS_DISCORD_PRESENCE_SUPPORTED
	shutdown();

	if (!validApplicationId(id)) {
		sendToLog(LogLevel::Warn, "Discord Rich Presence disabled: invalid Discord application ID\n");
		return false;
	}

	applicationId     = id;
	startTimestamp    = static_cast<long long>(std::time(nullptr));
	nextServiceMs     = 0;
	nextRefreshMs     = 0;
	nextReconnectMs   = 0;
	nonceCounter      = 0;
	enabled           = true;
	hasActivity       = false;
	unavailableLogged = false;
	failureLogged     = false;
	updateLogged      = false;

	return connect();
#else
	(void)id;
	return false;
#endif
}

void DiscordPresence::update(const Activity &activity) {
#ifdef ONS_DISCORD_PRESENCE_SUPPORTED
	if (!enabled)
		return;
	currentActivity = activity;
	hasActivity     = true;
	sendActivity(&activity);
#else
	(void)activity;
#endif
}

void DiscordPresence::service() {
#ifdef ONS_DISCORD_PRESENCE_SUPPORTED
	if (!enabled)
		return;

	const uint64_t now = nowMilliseconds();
	if (now < nextServiceMs)
		return;
	nextServiceMs = now + ServiceIntervalMs;

	if (!connected) {
		if (now < nextReconnectMs)
			return;
		nextReconnectMs = now + ReconnectIntervalMs;
		if (connect() && hasActivity)
			sendActivity(&currentActivity);
		return;
	}

	for (int i = 0; i < 4 && connected && hasReadableData(); i++) {
		uint32_t opcode = 0;
		std::string payload;
		if (!readFrame(opcode, payload, 10)) {
			closeConnection();
			nextReconnectMs = nowMilliseconds() + ReconnectIntervalMs;
			return;
		}
		if (!handleFrame(opcode, payload))
			return;
	}

	if (connected && hasActivity && now >= nextRefreshMs)
		sendActivity(&currentActivity);
#endif
}

void DiscordPresence::shutdown() {
#ifdef ONS_DISCORD_PRESENCE_SUPPORTED
	if (connected)
		sendActivity(nullptr);
	closeConnection();
	applicationId.clear();
	enabled     = false;
	hasActivity = false;
#endif
}

#ifdef ONS_DISCORD_PRESENCE_SUPPORTED
bool DiscordPresence::connect() {
	if (connected)
		return true;

	if (!openConnection()) {
		if (!unavailableLogged) {
			sendToLog(LogLevel::Info, "Discord Rich Presence unavailable: Discord desktop IPC was not found\n");
			unavailableLogged = true;
		}
		return false;
	}

	std::string handshake = "{\"v\":1,\"client_id\":\"" + jsonEscaped(applicationId) + "\"}";
	if (!writeFrame(OpcodeHandshake, handshake)) {
		closeConnection();
		return false;
	}

	uint32_t opcode = 0;
	std::string payload;
	if (!readFrame(opcode, payload, 750) || opcode != OpcodeFrame || payload.find("\"evt\":\"READY\"") == std::string::npos) {
		if (!failureLogged) {
			sendToLog(LogLevel::Warn, "Discord Rich Presence handshake failed\n");
			failureLogged = true;
		}
		closeConnection();
		return false;
	}

	connected       = true;
	nextReconnectMs = 0;
	sendToLog(LogLevel::Info, "Discord Rich Presence connected\n");
	return true;
}

bool DiscordPresence::openConnection() {
#ifdef WIN32
	for (int i = 0; i < 10; i++) {
		for (const char *prefix : {"\\\\?\\pipe\\discord-ipc-", "\\\\.\\pipe\\discord-ipc-"}) {
			std::string path = std::string(prefix) + std::to_string(i);
			HANDLE handle    = CreateFileA(path.c_str(), GENERIC_READ | GENERIC_WRITE, 0, nullptr, OPEN_EXISTING, 0, nullptr);
			if (handle == INVALID_HANDLE_VALUE && GetLastError() == ERROR_PIPE_BUSY) {
				WaitNamedPipeA(path.c_str(), 50);
				handle = CreateFileA(path.c_str(), GENERIC_READ | GENERIC_WRITE, 0, nullptr, OPEN_EXISTING, 0, nullptr);
			}
			if (handle != INVALID_HANDLE_VALUE) {
				DWORD mode = PIPE_READMODE_BYTE;
				SetNamedPipeHandleState(handle, &mode, nullptr, nullptr);
				connection = handle;
				return true;
			}
		}
	}
	return false;
#elif defined(LINUX) || defined(MACOSX)
	for (const std::string &root : discordIpcSearchRoots()) {
		for (int i = 0; i < 10; i++) {
			std::string path = root + "/discord-ipc-" + std::to_string(i);

			int fd = socket(AF_UNIX, SOCK_STREAM, 0);
			if (fd < 0)
				continue;

			sockaddr_un addr{};
			if (path.size() >= sizeof(addr.sun_path)) {
				close(fd);
				continue;
			}
			addr.sun_family = AF_UNIX;
			std::strncpy(addr.sun_path, path.c_str(), sizeof(addr.sun_path) - 1);
#ifdef MACOSX
			addr.sun_len = static_cast<unsigned char>(sizeof(addr));
#endif

			if (::connect(fd, reinterpret_cast<sockaddr *>(&addr), sizeof(addr)) == 0) {
				connection = fd;
				return true;
			}
			close(fd);
		}
	}
	return false;
#else
	return false;
#endif
}

void DiscordPresence::closeConnection() {
#ifdef WIN32
	if (connection) {
		CloseHandle(static_cast<HANDLE>(connection));
		connection = nullptr;
	}
#elif defined(LINUX) || defined(MACOSX)
	if (connection >= 0) {
		close(connection);
		connection = -1;
	}
#endif
	connected = false;
}

bool DiscordPresence::hasReadableData() {
#ifdef WIN32
	DWORD available = 0;
	if (!PeekNamedPipe(static_cast<HANDLE>(connection), nullptr, 0, nullptr, &available, nullptr)) {
		closeConnection();
		return false;
	}
	return available > 0;
#elif defined(LINUX) || defined(MACOSX)
	timeval tv{};
	fd_set fds;
	FD_ZERO(&fds);
	FD_SET(connection, &fds);
	int ready = select(connection + 1, &fds, nullptr, nullptr, &tv);
	if (ready < 0 && errno == EINTR)
		return false;
	if (ready < 0) {
		closeConnection();
		return false;
	}
	return ready > 0;
#else
	return false;
#endif
}

bool DiscordPresence::writeFrame(uint32_t opcode, const std::string &payload) {
	if (payload.size() > MaxFramePayload)
		return false;

	std::vector<uint8_t> frame(8 + payload.size());
	writeLE32(frame.data(), opcode);
	writeLE32(frame.data() + 4, static_cast<uint32_t>(payload.size()));
	if (!payload.empty())
		std::memcpy(frame.data() + 8, payload.data(), payload.size());

	return writeAll(frame.data(), frame.size());
}

bool DiscordPresence::readFrame(uint32_t &opcode, std::string &payload, uint32_t timeoutMs) {
	uint8_t header[8]{};
	if (!readExact(header, sizeof(header), timeoutMs))
		return false;

	opcode        = readLE32(header);
	uint32_t size = readLE32(header + 4);
	if (size > MaxFramePayload)
		return false;

	payload.assign(size, '\0');
	if (size == 0)
		return true;

	return readExact(reinterpret_cast<uint8_t *>(payload.data()), size, timeoutMs);
}

bool DiscordPresence::writeAll(const uint8_t *data, size_t size) {
#ifdef WIN32
	size_t writtenTotal = 0;
	while (writtenTotal < size) {
		DWORD written = 0;
		if (!WriteFile(static_cast<HANDLE>(connection), data + writtenTotal, static_cast<DWORD>(size - writtenTotal), &written, nullptr) || written == 0)
			return false;
		writtenTotal += written;
	}
	return true;
#elif defined(LINUX) || defined(MACOSX)
	size_t writtenTotal = 0;
	while (writtenTotal < size) {
		ssize_t written = send(connection, data + writtenTotal, size - writtenTotal, 0);
		if (written < 0 && errno == EINTR)
			continue;
		if (written <= 0)
			return false;
		writtenTotal += static_cast<size_t>(written);
	}
	return true;
#else
	(void)data;
	(void)size;
	return false;
#endif
}

bool DiscordPresence::readExact(uint8_t *data, size_t size, uint32_t timeoutMs) {
#ifdef WIN32
	size_t readTotal  = 0;
	uint64_t deadline = nowMilliseconds() + timeoutMs;
	while (readTotal < size) {
		DWORD available = 0;
		if (!PeekNamedPipe(static_cast<HANDLE>(connection), nullptr, 0, nullptr, &available, nullptr))
			return false;
		if (available < size - readTotal) {
			if (nowMilliseconds() >= deadline)
				return false;
			Sleep(5);
			continue;
		}

		DWORD bytesRead = 0;
		if (!ReadFile(static_cast<HANDLE>(connection), data + readTotal, static_cast<DWORD>(size - readTotal), &bytesRead, nullptr) || bytesRead == 0)
			return false;
		readTotal += bytesRead;
	}
	return true;
#elif defined(LINUX) || defined(MACOSX)
	size_t readTotal  = 0;
	uint64_t deadline = nowMilliseconds() + timeoutMs;
	while (readTotal < size) {
		uint64_t now = nowMilliseconds();
		if (now >= deadline)
			return false;

		timeval tv{};
		uint64_t remaining = deadline - now;
		tv.tv_sec          = static_cast<long>(remaining / 1000);
		tv.tv_usec         = static_cast<suseconds_t>((remaining % 1000) * 1000);

		fd_set fds;
		FD_ZERO(&fds);
		FD_SET(connection, &fds);
		int ready = select(connection + 1, &fds, nullptr, nullptr, &tv);
		if (ready < 0 && errno == EINTR)
			continue;
		if (ready <= 0)
			return false;

		ssize_t bytesRead = recv(connection, data + readTotal, size - readTotal, 0);
		if (bytesRead < 0 && errno == EINTR)
			continue;
		if (bytesRead <= 0)
			return false;
		readTotal += static_cast<size_t>(bytesRead);
	}
	return true;
#else
	(void)data;
	(void)size;
	(void)timeoutMs;
	return false;
#endif
}

bool DiscordPresence::handleFrame(uint32_t opcode, const std::string &payload) {
	if (opcode == OpcodePing) {
		if (!writeFrame(OpcodePong, payload)) {
			closeConnection();
			nextReconnectMs = nowMilliseconds() + ReconnectIntervalMs;
			return false;
		}
		return true;
	}
	if (opcode == OpcodeClose) {
		closeConnection();
		nextReconnectMs = nowMilliseconds() + ReconnectIntervalMs;
		return false;
	}
	if (payload.find("\"evt\":\"ERROR\"") != std::string::npos && !failureLogged) {
		sendToLog(LogLevel::Warn, "Discord Rich Presence update failed: %s\n", payload.c_str());
		failureLogged = true;
	}
	return true;
}

void DiscordPresence::sendActivity(const Activity *activity) {
	if (!connect())
		return;

	std::string payload = "{\"cmd\":\"SET_ACTIVITY\",\"args\":{\"pid\":" + std::to_string(currentProcessId());
	if (activity) {
		payload += ",\"activity\":{\"type\":0";
		if (!activity->details.empty())
			payload += ",\"details\":\"" + jsonEscaped(activity->details) + "\"";
		if (!activity->state.empty())
			payload += ",\"state\":\"" + jsonEscaped(activity->state) + "\"";
		if (startTimestamp > 0)
			payload += ",\"timestamps\":{\"start\":" + std::to_string(startTimestamp) + "}";
		if (!activity->largeImage.empty()) {
			payload += ",\"assets\":{\"large_image\":\"" + jsonEscaped(activity->largeImage) + "\"";
			if (!activity->largeText.empty())
				payload += ",\"large_text\":\"" + jsonEscaped(activity->largeText) + "\"";
			payload += "}";
		}
		payload += ",\"instance\":false}";
	} else {
		payload += ",\"activity\":null";
	}
	payload += "},\"nonce\":\"" + nextNonce() + "\"}";

	if (!writeFrame(OpcodeFrame, payload)) {
		closeConnection();
		nextReconnectMs = nowMilliseconds() + ReconnectIntervalMs;
		return;
	}
	nextRefreshMs = nowMilliseconds() + RefreshIntervalMs;

	uint32_t opcode = 0;
	std::string response;
	if (!readFrame(opcode, response, 250))
		return;
	if (!handleFrame(opcode, response))
		return;
	if (activity && response.find("\"cmd\":\"SET_ACTIVITY\"") != std::string::npos &&
	    response.find("\"evt\":null") != std::string::npos && !updateLogged) {
		sendToLog(LogLevel::Info, "Discord Rich Presence activity accepted\n");
		updateLogged = true;
	}
}

std::string DiscordPresence::nextNonce() {
	return "onscripter-new-" + std::to_string(++nonceCounter);
}
#endif
