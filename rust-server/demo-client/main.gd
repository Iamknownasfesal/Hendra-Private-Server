# A window that draws what the Rust server sends.
#
# Not the real game client — that still speaks the old protocol. This exists so the new stack can be
# watched rather than only asserted about: content and map loaded by the Rust server, the world
# ticked at twenty per second, snapshots encoded as deltas, carried over QUIC, decoded by the native
# extension, and drawn here.
#
# Arrow keys or WASD to move. The server decides where you actually end up.

extends Node2D

const TILE := 22.0
const SERVER_HOST := "localhost"
const SERVER_PORT := 2052

var conn
var me := -1
var world_name := ""
var status_text := "connecting…"
var chat_lines: Array[String] = []

var snapshots := 0
var snapshot_rate := 0.0
var _rate_window := 0.0
var _rate_count := 0
var _last_revision := 0

# Where we think we are, so input can be sent before the first snapshot lands.
var desired := Vector2.ZERO
var camera := Vector2.ZERO

# `--screenshot <path> [seconds]` captures a frame and quits, so a change to the drawing can be
# checked by looking at it rather than by reasoning about it.
var _shot_path := ""
var _shot_after := 3.0
var _elapsed := 0.0


func _ready() -> void:
	if not ClassDB.class_exists("HendraConnection"):
		status_text = "the native extension did not load — build it with: cargo build -p hendra-godot --release"
		push_error(status_text)
		return

	var args := OS.get_cmdline_user_args()
	for i in args.size():
		if args[i] == "--screenshot" and i + 1 < args.size():
			_shot_path = args[i + 1]
			if i + 2 < args.size():
				_shot_after = float(args[i + 2])

	conn = ClassDB.instantiate("HendraConnection")
	add_child(conn)
	# allow_any_certificate: the development server generates a self-signed certificate.
	conn.connect_to_server(SERVER_HOST, SERVER_PORT, "viewer", 1, true)


func _process(delta: float) -> void:
	if conn == null:
		return

	_drain_events()
	_track_snapshot_rate(delta)
	_send_input()
	queue_redraw()

	_elapsed += delta
	if _shot_path != "" and _elapsed >= _shot_after:
		# One frame after the redraw, so the capture holds what was just drawn.
		await RenderingServer.frame_post_draw
		get_viewport().get_texture().get_image().save_png(_shot_path)
		print("screenshot written to ", _shot_path)
		get_tree().quit()


func _drain_events() -> void:
	for event in conn.poll():
		match event["kind"]:
			"connected":
				status_text = "connected"
			"welcome":
				me = event["player"]
				world_name = event["world"]
				status_text = "in %s" % world_name
			"chat":
				chat_lines.append("%s: %s" % [event["from"], event["text"]])
				if chat_lines.size() > 6:
					chat_lines.pop_front()
			"rejected":
				status_text = "refused: %s" % event["reason_name"]
			"disconnected":
				status_text = "disconnected: %s" % event["reason"]


func _track_snapshot_rate(delta: float) -> void:
	var revision: int = conn.world_revision()
	if revision != _last_revision:
		_last_revision = revision
		snapshots += 1
		_rate_count += 1

	_rate_window += delta
	if _rate_window >= 1.0:
		snapshot_rate = _rate_count / _rate_window
		_rate_count = 0
		_rate_window = 0.0


func _send_input() -> void:
	if me == -1:
		return

	var here := _position_of(me)
	if here != Vector2.ZERO:
		camera = camera.lerp(here, 0.2) if camera != Vector2.ZERO else here
		desired = here

	var step := Vector2.ZERO
	if Input.is_key_pressed(KEY_LEFT) or Input.is_key_pressed(KEY_A):
		step.x -= 1.0
	if Input.is_key_pressed(KEY_RIGHT) or Input.is_key_pressed(KEY_D):
		step.x += 1.0
	if Input.is_key_pressed(KEY_UP) or Input.is_key_pressed(KEY_W):
		step.y -= 1.0
	if Input.is_key_pressed(KEY_DOWN) or Input.is_key_pressed(KEY_S):
		step.y += 1.0

	# Claim a position a little ahead. The server trims it to what our speed allows, which is why
	# holding a key does not accelerate us.
	if step != Vector2.ZERO:
		desired += step.normalized() * 0.5

	conn.send_input(desired.x, desired.y, Time.get_ticks_msec())


func _position_of(entity_id: int) -> Vector2:
	var ids: PackedInt32Array = conn.entity_ids()
	var positions: PackedFloat32Array = conn.entity_positions()
	for i in ids.size():
		if ids[i] == entity_id:
			return Vector2(positions[i * 2], positions[i * 2 + 1])
	return Vector2.ZERO


func _to_screen(world_position: Vector2) -> Vector2:
	var middle := get_viewport_rect().size * 0.5
	return middle + (world_position - camera) * TILE


func _draw() -> void:
	var size := get_viewport_rect().size
	draw_rect(Rect2(Vector2.ZERO, size), Color(0.07, 0.08, 0.10))

	if conn == null:
		_draw_text(Vector2(20, 40), status_text, Color(0.9, 0.5, 0.4), 16)
		return

	_draw_grid(size)

	var ids: PackedInt32Array = conn.entity_ids()
	var types: PackedInt32Array = conn.entity_types()
	var positions: PackedFloat32Array = conn.entity_positions()
	var hp: PackedInt32Array = conn.entity_hp()
	var max_hp: PackedInt32Array = conn.entity_max_hp()

	for i in ids.size():
		var at := _to_screen(Vector2(positions[i * 2], positions[i * 2 + 1]))
		if at.x < -40 or at.y < -40 or at.x > size.x + 40 or at.y > size.y + 40:
			continue

		var mine: bool = ids[i] == me
		var colour := Color(0.35, 0.78, 0.80)
		if mine:
			colour = Color(1.0, 0.78, 0.30)
		elif max_hp[i] > 0:
			colour = Color(0.82, 0.45, 0.32)

		draw_circle(at, 7.0 if mine else 5.0, colour)
		if mine:
			draw_arc(at, 12.0, 0, TAU, 24, Color(1.0, 0.78, 0.30, 0.5), 1.5)

		# A health bar for anything that can be hurt.
		if max_hp[i] > 0 and hp[i] < max_hp[i]:
			var fraction := float(hp[i]) / float(max_hp[i])
			draw_rect(Rect2(at + Vector2(-9, -14), Vector2(18, 3)), Color(0.2, 0.2, 0.2))
			draw_rect(Rect2(at + Vector2(-9, -14), Vector2(18 * fraction, 3)), Color(0.4, 0.8, 0.4))

		if not mine and types[i] != 0:
			_draw_text(at + Vector2(9, 4), "0x%x" % types[i], Color(0.45, 0.5, 0.55), 9)

	_draw_hud(size, ids.size())


func _draw_grid(size: Vector2) -> void:
	# A faint tile grid, aligned to world coordinates, so movement reads as movement.
	var faint := Color(1, 1, 1, 0.045)
	var middle := size * 0.5
	var first_x := fmod(middle.x - camera.x * TILE, TILE)
	var first_y := fmod(middle.y - camera.y * TILE, TILE)

	var x := first_x
	while x < size.x:
		draw_line(Vector2(x, 0), Vector2(x, size.y), faint, 1.0)
		x += TILE

	var y := first_y
	while y < size.y:
		draw_line(Vector2(0, y), Vector2(size.x, y), faint, 1.0)
		y += TILE


func _draw_hud(size: Vector2, visible_count: int) -> void:
	var ink := Color(0.85, 0.88, 0.92)
	var soft := Color(0.55, 0.60, 0.68)

	draw_rect(Rect2(Vector2(0, 0), Vector2(size.x, 30)), Color(0, 0, 0, 0.45))
	_draw_text(Vector2(14, 20), "Hendra — %s" % status_text, ink, 14)

	var right := "%d entities in sight   %.1f snapshots/sec   %d ms rtt" % [
		visible_count, snapshot_rate, conn.rtt_ms()
	]
	_draw_text(Vector2(size.x - 340, 20), right, soft, 13)

	_draw_text(Vector2(14, size.y - 52), "arrows or WASD to move", soft, 12)
	_draw_text(
		Vector2(14, size.y - 34),
		"position (%.2f, %.2f)   entity %d" % [desired.x, desired.y, me],
		soft,
		12
	)

	var line_y := 56.0
	for line in chat_lines:
		_draw_text(Vector2(14, line_y), line, soft, 12)
		line_y += 16.0


func _draw_text(at: Vector2, text: String, colour: Color, size: int) -> void:
	var font := ThemeDB.fallback_font
	draw_string(font, at, text, HORIZONTAL_ALIGNMENT_LEFT, -1, size, colour)
