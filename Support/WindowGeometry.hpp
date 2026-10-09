/**
 *  WindowGeometry.hpp
 *  ONScripter-RU
 *
 *  Map fullscreen cursor positions back to the windowed rendering area.
 *
 *  Consult LICENSE file for licensing terms and copyright holders.
 */

#pragma once

inline int fullscreenToWindowedCoordinate(int coordinate, int fullscreenSize, int windowedSize, int scriptSize, int scriptOffset) {
	return static_cast<int>((coordinate / static_cast<float>(fullscreenSize) - scriptOffset / static_cast<float>(scriptSize)) * windowedSize);
}
