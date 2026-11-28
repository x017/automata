"""Audio Source components and the script API that FPS-style games need:
entity.audio, ctx.audio, ctx.physics.raycast, ctx.input.mouse_delta,
mouse lock and ctx.hud (all headless)."""

import struct

import pytest

import automata as at


def write_wav(path, seconds=0.2):
    """A silent 16-bit mono WAV."""
    samples = int(44100 * seconds)
    data = b"\x00\x00" * samples
    header = b"RIFF" + struct.pack("<I", 36 + len(data)) + b"WAVEfmt "
    header += struct.pack("<IHHIIHH", 16, 1, 1, 44100, 88200, 2, 16)
    header += b"data" + struct.pack("<I", len(data))
    path.write_bytes(header + data)


def test_audio_source_components_validate_and_round_trip(tmp_path):
    src = at.AudioSource("sounds\\hum.ogg", volume=0.5, looping=True, spatial=True, range=12)
    assert src.clip == "sounds/hum.ogg"
    assert src.volume == pytest.approx(0.5)
    assert src.looping and src.spatial and not src.play_on_start
    assert src.range == pytest.approx(12)
    assert "hum.ogg" in repr(src)
    with pytest.raises(ValueError):
        at.AudioSource("a.ogg", volume=-1)
    with pytest.raises(ValueError):
        at.AudioSource("a.ogg", pitch=0)

    at.Scene().add(at.Entity("Radio").add(at.Transform()).add(src)).save(str(tmp_path / "s.ron"))
    text = (tmp_path / "s.ron").read_text()
    assert "audio_source" in text and "sounds/hum.ogg" in text
    radio = next(e for e in at.Scene.load(str(tmp_path / "s.ron")).entities() if e.name == "Radio")
    back = next(c for c in radio.components() if isinstance(c, at.AudioSource))
    assert back.looping and back.range == pytest.approx(12)


SCRIPT = '''
def start(entity, ctx):
    entity.state["frames"] = 0
    ctx.input.lock_mouse()


def update(entity, ctx):
    s = entity.state
    s["frames"] += 1
    if s["frames"] == 2:
        entity.audio.play()
        ctx.audio.play("zap.wav", volume=0.5, position=entity)
        ctx.audio.play("missing.wav")
    if s["frames"] == 4:
        entity.attributes["playing"] = entity.audio.playing
        entity.attributes["locked"] = ctx.input.mouse_locked
        entity.attributes["delta"] = sum(abs(v) for v in ctx.input.mouse_delta)
        hit = ctx.physics.raycast(entity.transform.position, (0, 0, -1), 50, ignore=entity)
        entity.attributes["hit"] = hit.entity.name if hit else ""
        entity.attributes["hit_distance"] = hit.distance if hit else -1.0
        miss = ctx.physics.raycast(entity.transform.position, (0, 1, 0), 50, ignore=entity)
        entity.attributes["miss"] = miss is None
        ctx.hud.image("crosshair.png", *ctx.hud.center, size=(32, 32))
        ctx.hud.rect(100, 20, 180, 12, (1, 0, 0, 1))
        ctx.hud.text("100%", 40, 40, size=26, shadow=True)
        ctx.hud.text("CLEAR\\nnext", *ctx.hud.center, align="center", outline=(0, 0, 0, 1))
        w, h = ctx.screen.measure_text("100%", 26)
        entity.attributes["text_w"] = w
        entity.attributes["text_h"] = h
        entity.attributes["hud"] = "%dx%d" % ctx.hud.size
        if entity.audio.source is not None:
            entity.attributes["clip"] = entity.audio.source.clip
'''


def test_scripts_play_sounds_raycast_and_draw_the_hud(tmp_path, capfd):
    write_wav(tmp_path / "zap.wav", 0.05)
    write_wav(tmp_path / "hum.wav", 2.0)
    (tmp_path / "player.py").write_text(SCRIPT)
    player = (
        at.Entity("Player")
        .add(at.Transform(position=(0, 1, 0)))
        .add(at.RigidBody.fixed())
        .add(at.Collider3D.ball(0.4))
        .add(at.AudioSource("hum.wav", looping=True))
    )
    player.script = "player.py"
    scene = (
        at.Scene()
        .add(player)
        .add(
            at.Entity("Target")
            .add(at.Transform(position=(0, 1, -6)))
            .add(at.RigidBody.fixed())
            .add(at.Collider3D.cuboid(2, 2, 2))
        )
        .add(
            at.Entity("Music")
            .add(at.Transform())
            .add(at.AudioSource("hum.wav", play_on_start=True, looping=True))
        )
    )
    scene.save(str(tmp_path / "level.ron"))

    seen = {}

    def after(ctx):
        seen.update(dict(ctx.scene["Player"].attributes))

    at.run_scene(str(tmp_path / "level.ron"), headless=True, max_frames=6, update=after)
    assert seen["playing"] is True
    assert seen["locked"] is True
    assert seen["delta"] == 0.0
    assert seen["hit"] == "Target"
    assert seen["hit_distance"] == pytest.approx(5.0, abs=1e-3)
    assert seen["miss"] is True
    assert seen["hud"] == "1280x720"
    assert 30 < seen["text_w"] < 80 and abs(seen["text_h"] - 26) < 0.01
    assert seen["clip"] == "hum.wav"
    err = capfd.readouterr().err
    assert "missing.wav" in err
    assert err.count("Couldn't play missing.wav") == 1
