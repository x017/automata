#!/usr/bin/env python3
"""Regenerate the tiny original WAV effects used by oop_fps.py."""

import math
import random
import struct
import wave
from pathlib import Path

OUT = Path(__file__).resolve().parent
RATE = 22050


def save(name, samples, *, loop=False):
    path = OUT / name
    with wave.open(str(path), "wb") as stream:
        stream.setnchannels(1)
        stream.setsampwidth(2)
        stream.setframerate(RATE)
        stream.writeframes(b"".join(struct.pack("<h", int(max(-1.0, min(1.0, s)) * 32767)) for s in samples))
    return path


def tone(duration, fn, envelope):
    count = int(RATE * duration)
    return [fn(i / RATE, i / max(1, count - 1)) * envelope(i / max(1, count - 1)) for i in range(count)]


def noise(duration, seed=7):
    random.seed(seed)
    return [random.uniform(-1.0, 1.0) for _ in range(int(RATE * duration))]


def mix(*tracks):
    size = max(map(len, tracks))
    return [sum(track[i] if i < len(track) else 0.0 for track in tracks) for i in range(size)]


def fade_out(t):
    return (1.0 - t) ** 3


def fade_in_out(t):
    return min(1.0, t * 30.0) * min(1.0, (1.0 - t) * 24.0)


# Punchy synthetic sci-fi rifle: low thump, noisy crack, and a short zap tail.
shot = mix(
    tone(0.24, lambda t, p: math.sin(2 * math.pi * (78 - 38 * p) * t), fade_out),
    [n * math.exp(-i / (RATE * 0.038)) * 0.45 for i, n in enumerate(noise(0.24, 19))],
    tone(0.13, lambda t, p: math.sin(2 * math.pi * (740 - 440 * p) * t), lambda p: math.exp(-p * 8) * 0.18),
)
save("rifle_shot.wav", [s * 0.76 for s in shot])

# Compact metal impacts and a descending, softened robot shutdown chirp.
hit = mix(
    tone(0.18, lambda t, p: math.sin(2 * math.pi * (190 - 80 * p) * t), fade_out),
    [n * math.exp(-i / (RATE * 0.025)) * 0.24 for i, n in enumerate(noise(0.18, 31))],
)
save("bot_hit.wav", [s * 0.65 for s in hit])
down = tone(0.62, lambda t, p: math.sin(2 * math.pi * (700 - 470 * p) * t) + 0.28 * math.sin(2 * math.pi * (1050 - 700 * p) * t), lambda p: math.sin(math.pi * p) * (1.0 - p) * 0.55)
save("bot_down.wav", down)

# Player damage cue: a quick warning beep pair.
beep = mix(
    tone(0.16, lambda t, p: math.sin(2 * math.pi * 520 * t), lambda p: math.sin(math.pi * p) * 0.48),
    tone(0.16, lambda t, p: math.sin(2 * math.pi * 390 * t), lambda p: math.sin(math.pi * p) * 0.33),
)
save("player_hit.wav", beep)

# A short mechanical click/ratchet for reloading.
reload_sound = mix(
    tone(0.33, lambda t, p: math.sin(2 * math.pi * (115 + 460 * p) * t), lambda p: math.exp(-p * 4) * 0.30),
    [n * math.exp(-i / (RATE * 0.045)) * 0.22 for i, n in enumerate(noise(0.33, 53))],
)
save("reload.wav", reload_sound)

# Looping low synth drone under the arena; seamless at the 4 second boundary.
ambient = []
for i in range(RATE * 4):
    t = i / RATE
    envelope = 0.55 + 0.12 * math.sin(2 * math.pi * t / 4.0)
    sample = (math.sin(2 * math.pi * 55 * t) + 0.38 * math.sin(2 * math.pi * 82.5 * t) + 0.20 * math.sin(2 * math.pi * 110 * t)) / 1.58
    ambient.append(sample * envelope * 0.20)
save("arena_hum.wav", ambient)

print(f"Wrote procedural WAV effects to {OUT}")
