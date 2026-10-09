/**
 *  Window.cpp
 *  ONScripter-RU
 *
 *  Operating system window abstraction.
 *
 *  Consult LICENSE file for licensing terms and copyright holders.
 */

#include "Engine/Components/Window.hpp"
#include "Support/WindowGeometry.hpp"
#include "Engine/Graphics/GPU.hpp"
#include "Engine/Graphics/Common.hpp"
#include "Engine/Core/ONScripter.hpp"

#include "Support/SDLCompat.hpp"
#ifdef WIN32
#include <windows.h>
#include "Support/SDLSysWMCompat.hpp"
#include "Resources/Support/WinRes.hpp"
#endif

#include <unordered_map>
#include <string>
#include <algorithm>

WindowController window;

int WindowController::ownInit() {
	auto system_offset_x_str = ons.ons_cfg_options.find("system-offset-x");
	if (system_offset_x_str != ons.ons_cfg_options.end())
		system_offset_x = std::stoi(system_offset_x_str->second);

	auto system_offset_y_str = ons.ons_cfg_options.find("system-offset-y");
	if (system_offset_y_str != ons.ons_cfg_options.end())
		system_offset_y = std::stoi(system_offset_y_str->second);

	if (ons.ons_cfg_options.contains("scale"))
		scaled_flag = true;

	if (ons.ons_cfg_options.contains("full-clip-limit"))
		fullscreen_reduce_clip = true;

	if (ons.ons_cfg_options.contains("fullscreen"))
		fullscreen_mode = true;

	return 0;
}

int WindowController::ownDeinit() {

	return 0;
}

bool WindowController::showMessageBox(uint32_t flags, const char *title, const char *message, int numbuttons, const SDL_MessageBoxButtonData *buttons, int &res) {
	SDL_MessageBoxData data{};
	data.flags       = flags;
	data.window      = window;
	data.title       = title;
	data.message     = message;
	data.numbuttons  = numbuttons;
	data.buttons     = buttons;
	data.colorScheme = nullptr;
	return onsShowMessageBox(&data, &res);
}

bool WindowController::showSimpleMessageBox(uint32_t flags, const char *title, const char *message) {
	return onsShowSimpleMessageBox(flags, title, message, window);
}

void WindowController::setMousePosition(int x, int y) {
	SDL_WarpMouseInWindow(window, x, y);
}

void WindowController::setMinimize(bool hide) {
	if (hide)
		SDL_MinimizeWindow(window);
	else
		SDL_RestoreWindow(window);
}

void WindowController::setActiveState(bool activate) {
	SDL_GL_MakeCurrent(window, activate ? glcontext : nullptr);
}

void WindowController::setMainTarget(RenderTarget *target) {
	window    = SDL_GetWindowFromID(target->context->windowID);
	glcontext = SDL_GL_GetCurrentContext();
	if (window && !current_title.empty())
		SDL_SetWindowTitle(window, current_title.c_str());
}

void WindowController::setTitle(const char *title) {
	const char *safeTitle = title ? title : "";
	if (current_title == safeTitle)
		return;
	current_title = safeTitle;
	if (window)
		SDL_SetWindowTitle(window, current_title.c_str());
}

void WindowController::setIcon(SDL_Surface *icon) {
	if (icon) {
		SDL_SetWindowIcon(window, icon);
		return;
	}

#if defined(WIN32) && !defined(ONS_USE_SDL3)
	//use the (first) Windows icon resource
	const HANDLE wicon[2]{
	    LoadImage(GetModuleHandle(nullptr), MAKEINTRESOURCE(ONSCRICON), IMAGE_ICON,
	              GetSystemMetrics(SM_CXICON), GetSystemMetrics(SM_CYICON), 0),
	    LoadImage(GetModuleHandle(nullptr), MAKEINTRESOURCE(ONSCRICON), IMAGE_ICON,
	              GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON), 0)};
	for (int i = 0; i < 2; i++) {
		if (wicon[i]) {
			SDL_SysWMinfo info;
			SDL_VERSION(&info.version);
			SDL_GetWindowWMInfo(window, &info);
			SendMessage(info.info.win.window, WM_SETICON, i == 1 ? ICON_SMALL : ICON_BIG, (LPARAM)wicon[i]);
		}
	}
#endif
}

void WindowController::translateRendering(float &x, float &y, RenderRect *&clip) {
	if (fullscreen_mode) {
		x += fullscript_offset_x;
		y += fullscript_offset_y;
		if (fullscreen_reduce_clip && !clip)
			clip = &fullscreen_reduced_clip;
	}
}

void WindowController::translateWindowToScriptCoords(int &x, int &y) {
	x = static_cast<int>(x * (script_width / static_cast<float>(screen_width)));
	y = static_cast<int>(y * (script_height / static_cast<float>(screen_height)));

	if (fullscreen_mode) {
		x -= fullscript_offset_x;
		y -= fullscript_offset_y;
	}
}

void WindowController::translateScriptToWindowCoords(int &x, int &y) {
	if (fullscreen_mode) {
		x = static_cast<int>(static_cast<float>(x + fullscript_offset_x) * (screen_width / static_cast<float>(script_width)));
		y = static_cast<int>(static_cast<float>(y + fullscript_offset_y) * (screen_height / static_cast<float>(script_height)));
	} else {
		x = static_cast<int>(static_cast<float>(x) * (screen_width / static_cast<float>(script_width)));
		y = static_cast<int>(static_cast<float>(y) * (screen_height / static_cast<float>(script_height)));
	}
}

void WindowController::applyDimensions(int rw, int rh, int cw, int ch, int dw) {
	script_width  = rw;
	script_height = rh;

	if (cw == 0 || ch == 0) {
		canvas_width  = script_width * 1.25;
		canvas_height = script_height * 1.25;
	} else {
		canvas_width  = cw;
		canvas_height = ch;
	}

	if (dw > 0) {
		screen_width  = dw;
		screen_height = dw * script_height / script_width;
	} else {
		screen_width  = script_width;
		screen_height = script_height;
	}

	windowed_screen_width  = screen_width;
	windowed_screen_height = screen_height;
}

float WindowController::currentDisplayRefreshRate() const {
	if (!window)
		return 0.0f;

#if defined(ONS_USE_SDL3)
	SDL_DisplayID display_id        = SDL_GetDisplayForWindow(window);
	const SDL_DisplayMode *mode_ptr = display_id ? SDL_GetCurrentDisplayMode(display_id) : nullptr;
	if (!mode_ptr && display_id)
		mode_ptr = SDL_GetDesktopDisplayMode(display_id);
	if (!mode_ptr)
		return 0.0f;
	if (mode_ptr->refresh_rate_numerator > 0 && mode_ptr->refresh_rate_denominator > 0)
		return static_cast<float>(mode_ptr->refresh_rate_numerator) / static_cast<float>(mode_ptr->refresh_rate_denominator);
	return mode_ptr->refresh_rate;
#else
	int display_index = SDL_GetWindowDisplayIndex(window);
	if (display_index < 0)
		return 0.0f;

	SDL_DisplayMode mode{};
	if (SDL_GetCurrentDisplayMode(display_index, &mode) != 0 &&
	    SDL_GetDesktopDisplayMode(display_index, &mode) != 0)
		return 0.0f;
	return static_cast<float>(mode.refresh_rate);
#endif
}

void WindowController::getInitialRenderSize(int &w, int &h) {
	w = screen_width;
	h = screen_height;
}

void WindowController::updateFullscreenGeometry(int display_width, int display_height) {
	if (display_width <= 0 || display_height <= 0 || script_width <= 0 || script_height <= 0)
		return;
	if (scaled_flag) {
		float scr_stretch_x = display_width / static_cast<float>(script_width);
		float scr_stretch_y = display_height / static_cast<float>(script_height);

		// This was marked "Deprecated and should be removed" -- now it only exists in this one place. The suspicious +0.5 makes me hesitant to refactor to remove this variable.
		int screen_ratio1, screen_ratio2;

		// Constrain aspect to same as game
		if (scr_stretch_x > scr_stretch_y) {
			screen_ratio1 = display_height;
			screen_ratio2 = script_height;
		} else {
			screen_ratio1 = display_width;
			screen_ratio2 = script_width;
		}

		fullscreen_width  = std::round(static_cast<float>(script_width * screen_ratio1) / screen_ratio2);
		fullscreen_height = std::round(static_cast<float>(script_height * screen_ratio1) / screen_ratio2);
	}

	fullscript_width    = script_width * display_width / static_cast<float>(fullscreen_width);
	fullscript_height   = script_height * display_height / static_cast<float>(fullscreen_height);
	fullscript_offset_x = (fullscript_width - script_width) / 2 - system_offset_x;
	fullscript_offset_y = (fullscript_height - script_height) / 2 - system_offset_y;
	// A hack for some resolutions to solve scaling issues like random stripes
	// e. g. 1366x768
	// bg white,1
	// lsp s0_1,"white1080p.png",0,0
	// print 1
	fullscreen_reduced_clip = {fullscript_offset_x + 0.5f, fullscript_offset_y + 0.5f, script_width - 1.0f, script_height - 1.0f};
}

bool WindowController::updateDisplayData(bool getpos) {
	if (getpos)
		SDL_GetWindowPosition(window, &window_x, &window_y);

	int displays{onsGetNumVideoDisplays()};
	displayData.clear();
	displayData.displays.resize(displays);
	displayData.displaysByArea.resize(displays);

	RenderRect windowRegion{static_cast<float>(window_x), static_cast<float>(window_y), static_cast<float>(screen_width), static_cast<float>(screen_height)};

	for (int d = 0; d < displays; d++) {
		SDL_DisplayMode video_mode{};
		SDL_Rect displayBounds{};
		bool haveDisplayBounds = onsGetDisplayBounds(d, &displayBounds);
		if (!onsGetDesktopDisplayMode(d, &video_mode)) {
			video_mode.w = haveDisplayBounds ? displayBounds.w : screen_width;
			video_mode.h = haveDisplayBounds ? displayBounds.h : screen_height;
		}
		auto &display         = displayData.displays[d];
		display.id            = d;
		display.native_width  = video_mode.w;
		display.native_height = video_mode.h;
		display.region        = haveDisplayBounds ? displayBounds : SDL_Rect{0, 0, video_mode.w, video_mode.h};

		// Determine the size of the portion of the window visible on this display
		SDL_Rect &r = display.region;
		RenderRect displayRegion{static_cast<float>(r.x), static_cast<float>(r.y), static_cast<float>(r.w), static_cast<float>(r.h)};
		RenderRect visibleWindowRegion = windowRegion;
		int area                     = doClipping(&visibleWindowRegion, &displayRegion) ? 0 :
		                                                              static_cast<int>(visibleWindowRegion.w) * static_cast<int>(visibleWindowRegion.h);
		display.visibleArea = area;

		// Build the vector for later sorted-iteration (we want to try the displays in order of amount of window visible on screen)
		displayData.displaysByArea[d] = &display;
	}

	// Determine which display will be used for fullscreen.
	std::sort(displayData.displaysByArea.begin(), displayData.displaysByArea.end(), [](const Display *lhs, const Display *rhs) {
		return lhs->visibleArea >= rhs->visibleArea;
	});
	for (int d = 0; d < displays; d++) {
		Display *display = displayData.displaysByArea[d];
		if (!displayData.fullscreenDisplay) {
			if (scaled_flag) {
				// Scaling is on, so it fits no matter the display size.
				displayData.fullscreenDisplay = display;
			} else if (screen_width <= display->native_width && screen_height <= display->native_height) {
				// Only set this as a fullscreen display if it fits.
				displayData.fullscreenDisplay = display;
#if !defined(DROID)
				fullscreen_width              = screen_width;
				fullscreen_height             = screen_height;
#endif
			}
			// The fullscreen display is the first to satisfy these conditions.
		}
		//sendToLog(LogLevel::Info, "Display %u: %u x %u (visible area %u)\n", display->id, display->native_width, display->native_height, display->visibleArea);
	}

	if (!displayData.fullscreenDisplay)
		return false;

#if defined(DROID)
	// Android's surface can be smaller than its display in split-screen and
	// floating-window modes. applySurfaceGeometry() is the only owner of the
	// canvas there; writing display-derived geometry here was the divergence
	// that caused the resize bugs in the first place.
	return true;
#else
	updateFullscreenGeometry(displayData.fullscreenDisplay->native_width,
	                         displayData.fullscreenDisplay->native_height);

	return true;
#endif
}

#if defined(DROID)
void WindowController::applySurfaceGeometry() {
	// Android resizes the surface in place -- the manifest's configChanges
	// covers orientation and screenSize, so the activity is never recreated --
	// and the normal desktop display-data path is not run for that event.
	//
	// Left stale, fullscript_* keeps the aspect of the previous orientation:
	// a 1920x2688 portrait canvas presented into a 2800x2000 landscape
	// swapchain, which is the squashed full-width band.
	//
	// Derived from the window, not the display. The old shared path sized the
	// canvas from displayData.fullscreenDisplay->native_*, which is the whole
	// screen. Under split screen or a floating window the display does not change
	// while the window does, so it kept returning the fullscreen answer: a
	// 1918x1079 floating window -- almost exactly the game's own 16:9 -- was
	// handed a 1920x1371 canvas and squashed by 27%. The window size is correct
	// for every cause, rotation included.
	int winW = 0, winH = 0;
	SDL_GetWindowSizeInPixels(window, &winW, &winH);
	if (winW <= 0 || winH <= 0)
		return;

	// Largest whole-script scale that still fits, i.e. letterbox rather than
	// crop or stretch.
	const float scale = std::min(winW / static_cast<float>(script_width),
	                             winH / static_cast<float>(script_height));
	if (!(scale > 0.0f))
		return;

	fullscreen_width  = std::round(script_width * scale);
	fullscreen_height = std::round(script_height * scale);
	if (fullscreen_width <= 0 || fullscreen_height <= 0)
		return;

	// The whole window expressed in script units; the scene sits centred in it
	// and the remainder is the letterbox.
	fullscript_width    = script_width * winW / static_cast<float>(fullscreen_width);
	fullscript_height   = script_height * winH / static_cast<float>(fullscreen_height);
	fullscript_offset_x = (fullscript_width - script_width) / 2 - system_offset_x;
	fullscript_offset_y = (fullscript_height - script_height) / 2 - system_offset_y;
	fullscreen_reduced_clip = {fullscript_offset_x + 0.5f, fullscript_offset_y + 0.5f,
	                           script_width - 1.0f, script_height - 1.0f};

	screen_width  = fullscreen_width;
	screen_height = fullscreen_height;

	sendToLog(LogLevel::Info,
	          "Surface resize: window %dx%d, fullscreen %dx%d, fullscript %dx%d\n",
	          winW, winH, fullscreen_width, fullscreen_height, fullscript_width, fullscript_height);

	gpu.setVirtualResolution(fullscript_width, fullscript_height);
}
#endif

bool WindowController::changeMode(bool perform, bool correct, int mode) {
	// To my regret SDL & SDL_gpu fullscreen APIs are neither convenient, nor perfect.
	// This function needs some improvement I guess, because these clearWholeTargets shouldn't
	// be required on OS X, for example (though, the glitches show they are)
	// The current model is:
	// 1) Resize main window to display dimensions.
	// 2) Set up a new virtual resolution.
	// 3) Enter fullscreen mode.
	// Window positioning and mouse remaps are done in a manual manner here.

	if (perform && mode == 0 && fullscreen_mode) {
		int mouse_x, mouse_y;
		onsGetMouseState(&mouse_x, &mouse_y);
		windowed_mouse_position.x = fullscreenToWindowedCoordinate(mouse_x, screen_width, windowed_screen_width, script_width, fullscript_offset_x);
		windowed_mouse_position.y = fullscreenToWindowedCoordinate(mouse_y, screen_height, windowed_screen_height, script_height, fullscript_offset_y);
	}

	if (!updateDisplayData() && mode > 0) {
		// Request to enter fullscreen when we are in fullscreen-banned mode. Deny it
		return false;
	}

	if (perform && mode >= 0 && static_cast<int>(fullscreen_mode) != mode) {
		// Make sure all the blits are done and the screen is empty, before we continue
		gpu.clearWholeTarget(ons.screen_target);
		GPU_Flip(ons.screen_target);
		GPU_FlushBlitBuffer();

		if (mode == 1) {
#if defined(DROID)
			// applySurfaceGeometry() is the single owner of the canvas.
			// Deriving it here from display metrics as well is what let the two
			// disagree: on a desktop fullscreen means the window is the display,
			// but on Android it does not, so whichever path ran last won. The
			// rest of the desktop branch does not apply either -- the system
			// owns the surface position and size, and there is no pointer to
			// warp.
			applySurfaceGeometry();
			onsSetWindowFullscreen(window, true);
			ons.screen_target = GPU_GetContextTarget();
			fullscreen_mode   = true;
#else
			updateDisplayData(true); // window_x and window_y have changed, so our display data must be recalculated.
			screen_width  = fullscreen_width;
			screen_height = fullscreen_height;

			SDL_SetWindowPosition(window, displayData.fullscreenDisplay->region.x, displayData.fullscreenDisplay->region.y); // Move to make it look less offscreen
			GPU_SetWindowResolution(displayData.fullscreenDisplay->native_width, displayData.fullscreenDisplay->native_height);
			gpu.setVirtualResolution(fullscript_width, fullscript_height);

			int mouse_x, mouse_y;
			onsGetMouseState(&mouse_x, &mouse_y);
			//Fullscreen set
			onsSetWindowFullscreen(window, true);
			//We need to correct a shifted mouse
			//sendToLog(LogLevel::Info, "Going to fullscreen. Before: %u, %u\n", mouse_x, mouse_y);
			mouse_x = (mouse_x * fullscreen_width / windowed_screen_width) + ((screen_width / static_cast<float>(script_width)) * fullscript_offset_x);
			mouse_y = (mouse_y * fullscreen_height / windowed_screen_height) + ((screen_height / static_cast<float>(script_height)) * fullscript_offset_y);
			//sendToLog(LogLevel::Info, "Going to fullscreen. After: %u, %u\n", mouse_x, mouse_y);
			SDL_WarpMouseInWindow(window, mouse_x, mouse_y);
			ons.screen_target = GPU_GetContextTarget();
			fullscreen_mode   = true;
#endif
		} else {
			onsSetWindowFullscreen(window, false);
			ons.screen_target = GPU_GetContextTarget();
			fullscreen_mode   = false;
		}
		// On OS X mode changes are some animation that needs to be waited for, return when it ends
		fullscreen_needs_fix = true;
	} else if (perform && mode >= 0) {
		correct = false;
	}

	if (correct) {
#if !defined(DROID)
		if (fullscreen_mode) {
			int actual_width, actual_height;
			SDL_GetWindowSize(window, &actual_width, &actual_height);
			updateFullscreenGeometry(actual_width, actual_height);
			screen_width = fullscreen_width;
			screen_height = fullscreen_height;
			GPU_SetWindowResolution(actual_width, actual_height);
			gpu.setVirtualResolution(fullscript_width, fullscript_height);
		}
#endif
		// Set correct window dimensions (we are returning to windowed mode)
		if (!fullscreen_mode) {
			screen_width  = windowed_screen_width;
			screen_height = windowed_screen_height;

			int mouse_x, mouse_y;
			onsGetMouseState(&mouse_x, &mouse_y);
			//We need to correct a shifted mouse
			//sendToLog(LogLevel::Info, "Going to windowed. Before: %u, %u\n", mouse_x, mouse_y);
			mouse_x = windowed_mouse_position.x;
			mouse_y = windowed_mouse_position.y;
			//sendToLog(LogLevel::Info, "Going to windowed. After: %u, %u\n", mouse_x, mouse_y);

			GPU_SetWindowResolution(screen_width, screen_height);
			gpu.setVirtualResolution(script_width, script_height);

			if (fullscreen_needs_fix)
				SDL_SetWindowPosition(window, window_x, window_y);
			SDL_SetWindowSize(window, screen_width, screen_height);

			if (fullscreen_needs_fix)
				SDL_WarpMouseInWindow(window, mouse_x, mouse_y);
		}
		fullscreen_needs_fix = false;
		// mode change requires us to redraw the screen, when we are done
		gpu.clearWholeTarget(ons.screen_target);
#ifdef WIN32
		// Looks like old "don't respond to first Flip" bug is back
		GPU_Flip(ons.screen_target);
#endif
	}

	return correct;
}

bool WindowController::earlySetMode() {
	if (fullscreen_mode) {
		fullscreen_mode = false;
		return changeMode(true, true, 1);
	}

	// Unsure if true is needed, but just to make sure
	updateDisplayData(true);
	return false;
}
