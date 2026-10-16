-- T1-S flight computer link: the TX15 on its USB cable, in CRSF. A widget (the TX15 is a colour
-- radio, which has no telemetry-script screens).
--
-- One-time setup on the radio:
--   1. Hardware, USB Serial (VCP): LUA. When you plug the radio in, pick VCP.
--   2. Put this folder (WIDGETS/T1SFC) on the SD card.
--   3. Model, Screen setup: add a main view with a widget zone and choose T1SFC for it.
-- It runs in the background whichever view is up (a widget's background function runs always),
-- but the model has to be the one whose main views hold the widget.
--
-- Up the cable (to the sim's flight computer):
--   - the 16 channels of the model, as CRSF RC channels (type 0x16);
--   - the MAVLink telemetry that came down for the ground station, carried on (CRSF envelopes
--     0xAA from the radio, 0xEA), and the ground station's commands, carried on to the plane
--     (envelopes to the flight controller, 0xC8).
-- Down the cable: attitude, battery, GPS, speed, climb, flight mode, link statistics and MAVLink
-- envelopes (with ArduPilot's own messages). The widget draws them in the layout of Yaapu's
-- ArduPilot telemetry script: the flight mode and ARMED state on top, an artificial horizon with
-- the speeds and the height beside it, the battery, GPS and home on the right, and ArduPilot's
-- messages along the bottom. The layout is in domains/avionics (the TX15 screen).
-- See domains/avionics.

local ADDR_FC, ADDR_RADIO, ADDR_TX = 0xC8, 0xEA, 0xEE
local T_RC, T_BATTERY, T_LINK, T_ATTITUDE, T_MODE, T_GPS, T_AIRSPEED, T_VARIO, T_ENVELOPE =
  0x16, 0x08, 0x14, 0x1E, 0x21, 0x02, 0x0A, 0x07, 0xAA

local crcTable = {
  -- CRC-8, polynomial 0xD5, for the byte 0 to 255 (index + 1: Lua tables start at 1)
  0, 213, 127, 170, 254, 43, 129, 84, 41, 252, 86, 131, 215, 2, 168, 125,
  82, 135, 45, 248, 172, 121, 211, 6, 123, 174, 4, 209, 133, 80, 250, 47,
  164, 113, 219, 14, 90, 143, 37, 240, 141, 88, 242, 39, 115, 166, 12, 217,
  246, 35, 137, 92, 8, 221, 119, 162, 223, 10, 160, 117, 33, 244, 94, 139,
  157, 72, 226, 55, 99, 182, 28, 201, 180, 97, 203, 30, 74, 159, 53, 224,
  207, 26, 176, 101, 49, 228, 78, 155, 230, 51, 153, 76, 24, 205, 103, 178,
  57, 236, 70, 147, 199, 18, 184, 109, 16, 197, 111, 186, 238, 59, 145, 68,
  107, 190, 20, 193, 149, 64, 234, 63, 66, 151, 61, 232, 188, 105, 195, 22,
  239, 58, 144, 69, 17, 196, 110, 187, 198, 19, 185, 108, 56, 237, 71, 146,
  189, 104, 194, 23, 67, 150, 60, 233, 148, 65, 235, 62, 106, 191, 21, 192,
  75, 158, 52, 225, 181, 96, 202, 31, 98, 183, 29, 200, 156, 73, 227, 54,
  25, 204, 102, 179, 231, 50, 152, 77, 48, 229, 79, 154, 206, 27, 177, 100,
  114, 167, 13, 216, 140, 89, 243, 38, 91, 142, 36, 241, 165, 112, 218, 15,
  32, 245, 95, 138, 222, 11, 161, 116, 9, 220, 118, 163, 247, 34, 136, 93,
  214, 3, 169, 124, 40, 253, 87, 130, 255, 42, 128, 85, 1, 212, 126, 171,
  132, 81, 251, 46, 122, 175, 5, 208, 173, 120, 210, 7, 83, 134, 44, 249,
}

local function crc8(s, first)
  local c = 0
  for i = first, #s do c = crcTable[bit32.bxor(c, string.byte(s, i)) + 1] end
  return c
end

-- One frame: address, length (type + payload + crc), type, payload, crc over type and payload.
local function frame(addr, kind, payload)
  local body = string.char(kind) .. payload
  return string.char(addr, #body + 1) .. body .. string.char(crc8(body, 1))
end

-- The 11-bit value ExpressLRS sends for a channel (-1024..1024 is 988..2012 us).
local function ticks(v)
  local t = math.floor(992 + v * 0.78125 + 0.5)
  if t < 0 then t = 0 elseif t > 2047 then t = 2047 end
  return t
end

local function channelsFrame()
  local bytes, acc, bits = {}, 0, 0
  for i = 1, 16 do
    acc = acc + bit32.lshift(ticks(getValue("ch" .. i)), bits)
    bits = bits + 11
    while bits >= 8 do
      bytes[#bytes + 1] = string.char(bit32.band(acc, 0xFF))
      acc = bit32.rshift(acc, 8)
      bits = bits - 8
    end
  end
  return frame(ADDR_FC, T_RC, table.concat(bytes))
end


local telemetry = {
  mode = "-", armed = false, volts = 0, amps = 0, used = 0, percent = 0, roll = 0, pitch = 0, quality = 0,
  airspeed = 0, groundspeed = 0, climb = 0, heading = 0, alt = 0, sats = 0, lat = 0, lon = 0,
  homeAlt = nil, homeLat = nil, homeLon = nil, last = 0, frames = 0, forwarded = 0,
}
local messages = {}          -- the last ArduPilot STATUSTEXT lines, newest last: { severity, text }
local mav = ""               -- MAVLink bytes from the plane, until whole messages are in them
local pending = ""
local nextSend = 0

local function be16(s, i)
  local v = string.byte(s, i) * 256 + string.byte(s, i + 1)
  if v >= 32768 then v = v - 65536 end
  return v
end

local function be32(s, i)
  local v = ((string.byte(s, i) * 256 + string.byte(s, i + 1)) * 256 + string.byte(s, i + 2)) * 256 + string.byte(s, i + 3)
  if v >= 2147483648 then v = v - 4294967296 end
  return v
end

-- STATUSTEXT (MAVLink message 253) is what ArduPilot says: PreArm checks, mode changes, failsafes.
local MAV_START = string.char(0xFD)
local function readMavlink(data)
  mav = mav .. data
  if #mav > 600 then mav = string.sub(mav, -300) end
  while true do
    local s = string.find(mav, MAV_START, 1, true)
    if not s then mav = ""; return end
    if s > 1 then mav = string.sub(mav, s); s = 1 end
    if #mav < 12 then return end
    local len = string.byte(mav, 2)
    local total = 12 + len + (bit32.band(string.byte(mav, 3), 1) ~= 0 and 13 or 0)
    if #mav < total then return end
    local id = string.byte(mav, 8) + 256 * string.byte(mav, 9) + 65536 * string.byte(mav, 10)
    if id == 253 and len >= 2 then
      local text = string.sub(mav, 12, 10 + len)
      local z = string.find(text, "\0", 1, true)
      if z then text = string.sub(text, 1, z - 1) end
      messages[#messages + 1] = { string.byte(mav, 11), text }
      if #messages > 4 then table.remove(messages, 1) end
    end
    mav = string.sub(mav, total + 1)
  end
end

local function handle(addr, kind, payload)
  telemetry.last = getTime()
  telemetry.frames = telemetry.frames + 1
  if kind == T_BATTERY and #payload >= 8 then
    telemetry.volts = be16(payload, 1) / 10
    telemetry.amps = be16(payload, 3) / 10
    telemetry.used = string.byte(payload, 5) * 65536 + string.byte(payload, 6) * 256 + string.byte(payload, 7)
    telemetry.percent = string.byte(payload, 8)
  elseif kind == T_ATTITUDE and #payload >= 6 then
    telemetry.pitch = be16(payload, 1) / 10000 * 57.2958
    telemetry.roll = be16(payload, 3) / 10000 * 57.2958
  elseif kind == T_MODE then
    local z = string.find(payload, "\0", 1, true)
    local text = z and string.sub(payload, 1, z - 1) or payload
    telemetry.armed = string.sub(text, -1) ~= "*"
    telemetry.mode = string.gsub(text, "%*$", "")
  elseif kind == T_LINK and #payload >= 3 then
    telemetry.quality = string.byte(payload, 3)
  elseif kind == T_GPS and #payload >= 15 then
    telemetry.lat = be32(payload, 1) / 1e7
    telemetry.lon = be32(payload, 5) / 1e7
    telemetry.groundspeed = (string.byte(payload, 9) * 256 + string.byte(payload, 10)) / 36
    telemetry.heading = (string.byte(payload, 11) * 256 + string.byte(payload, 12)) / 100
    telemetry.alt = string.byte(payload, 13) * 256 + string.byte(payload, 14) - 1000
    telemetry.sats = string.byte(payload, 15)
    -- Home is where the plane stood while it was disarmed.
    if not telemetry.armed then
      telemetry.homeAlt, telemetry.homeLat, telemetry.homeLon = telemetry.alt, telemetry.lat, telemetry.lon
    end
  elseif kind == T_AIRSPEED and #payload >= 2 then
    telemetry.airspeed = (string.byte(payload, 1) * 256 + string.byte(payload, 2)) / 36
  elseif kind == T_VARIO and #payload >= 2 then
    telemetry.climb = be16(payload, 1) / 10
  elseif kind == T_ENVELOPE then
    -- MAVLink that the plane sent (to the radio's module): carry it on for the ground station.
    -- What the ground station sent (to the radio): carry it on to the plane.
    local to = nil
    if addr == ADDR_TX then
      to = ADDR_RADIO
      readMavlink(string.sub(payload, 3, 2 + string.byte(payload, 2)))
    elseif addr == ADDR_RADIO then
      to = ADDR_FC
    end
    if to then
      serialWrite(frame(to, T_ENVELOPE, payload))
      telemetry.forwarded = telemetry.forwarded + 1
    end
  end
end

-- Cut the bytes that came down the cable into frames.
local function readLine()
  for _ = 1, 6 do                                       -- the line carries more than one read holds
    local got = serialRead(128)
    if not got or #got == 0 then break end
    pending = pending .. got
  end
  while #pending >= 4 do
    local addr, len = string.byte(pending, 1), string.byte(pending, 2)
    if (addr ~= ADDR_TX and addr ~= ADDR_RADIO and addr ~= ADDR_FC) or len < 2 or len > 62 then
      pending = string.sub(pending, 2)                  -- not a frame start: look from the next byte
    elseif #pending < len + 2 then
      return                                            -- the rest has not come yet
    else
      local body = string.sub(pending, 3, len + 1)      -- type and payload
      if crc8(body, 1) == string.byte(pending, len + 2) then
        handle(addr, string.byte(body, 1), string.sub(body, 2))
        pending = string.sub(pending, len + 3)
      else
        pending = string.sub(pending, 2)
      end
    end
  end
end


local function tick()
  local now = getTime()
  if now >= nextSend then
    serialWrite(channelsFrame())
    nextSend = now + 1                                  -- every tick the radio gives us (10 ms)
  end
  readLine()
end

local options = {}

local function create(zone, opts)
  return { zone = zone, options = opts }
end

local function update(widget, opts)
  widget.options = opts
end

local function background(widget)
  tick()
end

-- Colours. The radio has one custom-colour slot, so a colour is a { r, g, b } table and is put in
-- the slot just before each draw that uses it.
local WHITE, GREY, BG, PANEL = { 255, 255, 255 }, { 150, 160, 170 }, { 12, 16, 22 }, { 24, 30, 40 }
local SKY, EARTH, YELLOW, GREEN, RED, AMBER = { 40, 110, 200 }, { 120, 80, 30 }, { 255, 210, 0 }, { 60, 200, 90 }, { 230, 70, 60 }, { 240, 190, 60 }

local function paint(c)
  lcd.setColor(CUSTOM_COLOR, lcd.RGB(c[1], c[2], c[3]))
  return CUSTOM_COLOR
end
local function fill(x, y, w, h, c) lcd.drawFilledRectangle(x, y, w, h, paint(c)) end
local function text(x, y, str, size, c, align) lcd.drawText(x, y, str, size + (align or 0) + paint(c)) end
local function tri(x1, y1, x2, y2, x3, y3, c) lcd.drawFilledTriangle(x1, y1, x2, y2, x3, y3, paint(c)) end
local function disc(x, y, r, c) lcd.drawFilledCircle(x, y, r, paint(c)) end

-- A line two pixels thick (the radio's line is one).
local function thickLine(x1, y1, x2, y2, c)
  local p = paint(c)
  lcd.drawLine(x1, y1, x2, y2, SOLID, p)
  if math.abs(x2 - x1) >= math.abs(y2 - y1) then
    lcd.drawLine(x1, y1 + 1, x2, y2 + 1, SOLID, p)
  else
    lcd.drawLine(x1 + 1, y1, x2 + 1, y2, SOLID, p)
  end
end

-- A rectangle with rounded corners, filled.
local function roundRect(x, y, w, h, r, c)
  fill(x + r, y, w - 2 * r, h, c)
  fill(x, y + r, w, h - 2 * r, c)
  disc(x + r, y + r, r, c)
  disc(x + w - r - 1, y + r, r, c)
  disc(x + r, y + h - r - 1, r, c)
  disc(x + w - r - 1, y + h - r - 1, r, c)
end

-- Paints the corners of a box outside a quarter circle of radius r in the colour behind it, so what
-- was drawn square in the box has rounded corners, then draws the rounded outline, thick.
local function roundCorners(x, y, w, h, r, bg, edge)
  local corners = {
    { x, y, x + r, y + r, math.pi, 1.5 * math.pi },
    { x + w, y, x + w - r, y + r, 1.5 * math.pi, 2 * math.pi },
    { x + w, y + h, x + w - r, y + h - r, 0, 0.5 * math.pi },
    { x, y + h, x + r, y + h - r, 0.5 * math.pi, math.pi },
  }
  local steps = 6
  for _, k in ipairs(corners) do
    local ox, oy, cx, cy, a0, a1 = k[1], k[2], k[3], k[4], k[5], k[6]
    local px, py = cx + math.cos(a0) * r, cy + math.sin(a0) * r
    for i = 1, steps do
      local a = a0 + (a1 - a0) * i / steps
      local qx, qy = cx + math.cos(a) * r, cy + math.sin(a) * r
      tri(ox, oy, px, py, qx, qy, bg)
      thickLine(px, py, qx, qy, edge)
      px, py = qx, qy
    end
  end
  thickLine(x + r, y, x + w - r, y, edge)
  thickLine(x + r, y + h, x + w - r, y + h, edge)
  thickLine(x, y + r, x, y + h - r, edge)
  thickLine(x + w, y + r, x + w, y + h - r, edge)
end

local function distanceHome()
  local t = telemetry
  if not t.homeLat then return 0, 0 end
  local north = (t.lat - t.homeLat) * 111320
  local east = (t.lon - t.homeLon) * 111320 * math.cos(math.rad(t.homeLat))
  return math.sqrt(north * north + east * east), math.deg(math.atan2(east, north)) % 360
end

-- The ground half of the horizon: the HUD box cut by the horizon line, drawn as triangles.
-- (px, py) is a point on the line and (nx, ny) points down, towards the ground.
local function drawGround(x, y, w, h, px, py, nx, ny, c)
  local corners = { { x, y }, { x + w, y }, { x + w, y + h }, { x, y + h } }
  local poly = {}
  for i = 1, 4 do
    local a, b = corners[i], corners[i % 4 + 1]
    local da = (a[1] - px) * nx + (a[2] - py) * ny
    local db = (b[1] - px) * nx + (b[2] - py) * ny
    if da >= 0 then poly[#poly + 1] = a end
    if (da >= 0) ~= (db >= 0) then
      local t = da / (da - db)
      poly[#poly + 1] = { a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t }
    end
  end
  for i = 2, #poly - 1 do
    tri(poly[1][1], poly[1][2], poly[i][1], poly[i][2], poly[i + 1][1], poly[i + 1][2], c)
  end
end

-- The part of a line inside a box (Liang-Barsky), or nil.
local function clipLine(x1, y1, x2, y2, bx, by, bw, bh)
  local t0, t1, dx, dy = 0, 1, x2 - x1, y2 - y1
  for _, e in ipairs({ { -dx, x1 - bx }, { dx, bx + bw - x1 }, { -dy, y1 - by }, { dy, by + bh - y1 } }) do
    local p, q = e[1], e[2]
    if p == 0 then
      if q < 0 then return nil end
    else
      local r = q / p
      if p < 0 then
        if r > t1 then return nil end
        if r > t0 then t0 = r end
      else
        if r < t0 then return nil end
        if r < t1 then t1 = r end
      end
    end
  end
  return x1 + t0 * dx, y1 + t0 * dy, x1 + t1 * dx, y1 + t1 * dy
end

local function drawHud(x, y, w, h)
  local t = telemetry
  local cx, cy = x + w / 2, y + h / 2
  fill(x, y, w, h, SKY)
  local roll, pitch = math.rad(t.roll), -t.pitch                           -- (the radio showed it inverted)
  local k = h / 55                                                         -- pixels a degree
  local nx, ny = math.sin(roll), math.cos(roll)                            -- towards the ground
  local dx, dy = math.cos(roll), -math.sin(roll)                           -- along the horizon
  local px, py = cx + nx * pitch * k, cy + ny * pitch * k
  drawGround(x, y, w, h, px, py, nx, ny, EARTH)
  local ax, ay, bx, by = clipLine(px - dx * 2 * w, py - dy * 2 * w, px + dx * 2 * w, py + dy * 2 * w, x, y, w, h - 2)
  if ax then thickLine(ax, ay, bx, by, WHITE) end
  -- Pitch ladder: a line every 10 degrees, near the middle.
  for deg = -20, 20, 10 do
    if deg ~= 0 then
      local lx, ly = px - nx * deg * k, py - ny * deg * k
      if lx > x + 30 and lx < x + w - 30 and ly > y + 14 and ly < y + h - 14 then
        thickLine(lx - dx * 22, ly - dy * 22, lx + dx * 22, ly + dy * 22, WHITE)
        text(lx + dx * 24, ly + dy * 24 - 7, tostring(math.abs(deg)), SMLSIZE, WHITE)
      end
    end
  end
  -- The aircraft: a fixed mark in the middle.
  thickLine(cx - 34, cy, cx - 10, cy, YELLOW)
  thickLine(cx + 10, cy, cx + 34, cy, YELLOW)
  thickLine(cx - 10, cy, cx, cy + 8, YELLOW)
  thickLine(cx + 10, cy, cx, cy + 8, YELLOW)
  -- Roll: a pointer on an arc across the top.
  local r = h / 2 - 6
  for deg = -30, 30, 15 do
    local a = math.rad(deg)
    thickLine(cx + math.sin(a) * r, cy - math.cos(a) * r, cx + math.sin(a) * (r - 8), cy - math.cos(a) * (r - 8), WHITE)
  end
  local a = -roll
  tri(cx + math.sin(a) * (r - 8), cy - math.cos(a) * (r - 8),
    cx + math.sin(a - 0.07) * (r - 18), cy - math.cos(a - 0.07) * (r - 18),
    cx + math.sin(a + 0.07) * (r - 18), cy - math.cos(a + 0.07) * (r - 18), YELLOW)
  roundCorners(x, y, w, h, 14, BG, WHITE)
end

local function label(x, y, name, value, unit)
  text(x, y, name, SMLSIZE, GREY)
  text(x + 50, y - 4, value, MIDSIZE, WHITE)
  if unit then text(x + 50, y + 16, unit, SMLSIZE, GREY) end
end

local function refresh(widget, event, touchState)
  tick()
  local z = widget.zone
  local t = telemetry
  local live = getTime() - t.last < 100
  fill(z.x, z.y, z.w, z.h, BG)
  -- Top: the flight mode and whether it is armed, the battery, satellites and the link.
  roundRect(z.x + 2, z.y + 2, z.w - 4, 40, 10, PANEL)
  text(z.x + 14, z.y + 5, live and t.mode or "NO LINK", DBLSIZE, WHITE)
  if live then text(z.x + z.w * 0.34, z.y + 10, t.armed and "ARMED" or "DISARMED", MIDSIZE, t.armed and GREEN or RED) end
  text(z.x + z.w - 14, z.y + 14, string.format("GPS %d  LINK %d%%  %.1fV", t.sats, t.quality, t.volts), SMLSIZE, GREY, RIGHT)
  -- Middle: the horizon, the speeds on its left and the height on its right.
  local top = z.y + 48
  local hudH = z.h - 48 - 52
  local hudW = z.w * 0.44
  local hx = z.x + (z.w - hudW) / 2
  drawHud(hx, top, hudW, hudH)
  label(z.x + 8, top + 6, "AS", string.format("%.0f", t.airspeed), "m/s")
  label(z.x + 8, top + hudH * 0.36, "GS", string.format("%.0f", t.groundspeed), "m/s")
  label(z.x + 8, top + hudH * 0.68, "HDG", string.format("%03d", math.floor(t.heading)), nil)
  local home = t.homeAlt or t.alt
  label(hx + hudW + 10, top + 6, "ALT", string.format("%.0f", t.alt - home), "m")
  label(hx + hudW + 10, top + hudH * 0.36, "VS", string.format("%+.1f", t.climb), "m/s")
  local dist = distanceHome()
  label(hx + hudW + 10, top + hudH * 0.68, "HOME", string.format("%.0f", dist), "m")
  -- The battery, as a bar and the numbers.
  local bx, by = z.x + z.w - 78, top + hudH - 30
  text(bx, by - 8, string.format("%.1fA  %dmAh", t.amps, t.used), SMLSIZE, WHITE)
  roundRect(bx, by + 8, 68, 14, 5, GREY)
  roundRect(bx + 2, by + 10, 64, 10, 4, BG)
  local pct = math.max(0, math.min(100, t.percent))
  if pct > 4 then roundRect(bx + 2, by + 10, 64 * pct / 100, 10, 4, pct > 30 and GREEN or RED) end
  -- Bottom: what ArduPilot said last (PreArm checks, mode changes, failsafes).
  roundRect(z.x + 2, z.y + z.h - 48, z.w - 4, 46, 10, PANEL)
  local first = math.max(1, #messages - 2)
  for i = first, #messages do
    local m = messages[i]
    text(z.x + 12, z.y + z.h - 45 + (i - first) * 13, string.sub(m[2], 1, 60), SMLSIZE, m[1] <= 3 and RED or (m[1] <= 4 and AMBER or WHITE))
  end
end

return { name = "T1SFC", options = options, create = create, update = update, refresh = refresh, background = background }
