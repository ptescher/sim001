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
-- envelopes. The widget shows the main ones.
-- See domains/avionics.

local ADDR_FC, ADDR_RADIO, ADDR_TX = 0xC8, 0xEA, 0xEE
local T_RC, T_BATTERY, T_LINK, T_ATTITUDE, T_MODE, T_GPS, T_AIRSPEED, T_VARIO, T_ENVELOPE =
  0x16, 0x08, 0x14, 0x1E, 0x21, 0x02, 0x0A, 0x07, 0xAA

local crcTable = {}
local function buildCrc()
  for i = 0, 255 do
    local c = i
    for _ = 1, 8 do
      if bit32.band(c, 0x80) ~= 0 then c = bit32.band(bit32.bxor(bit32.lshift(c, 1), 0xD5), 0xFF)
      else c = bit32.band(bit32.lshift(c, 1), 0xFF) end
    end
    crcTable[i] = c
  end
end

local function crc8(s, first)
  local c = 0
  for i = first, #s do c = crcTable[bit32.bxor(c, string.byte(s, i))] end
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

local function be16(s, i)
  local v = string.byte(s, i) * 256 + string.byte(s, i + 1)
  if v >= 32768 then v = v - 65536 end
  return v
end

local telemetry = { mode = "-", armed = false, volts = 0, amps = 0, percent = 0, roll = 0, pitch = 0, quality = 0,
  speed = 0, climb = 0, last = 0, frames = 0, forwarded = 0 }
local pending = ""
local nextSend = 0

local function handle(addr, kind, payload)
  telemetry.last = getTime()
  telemetry.frames = telemetry.frames + 1
  if kind == T_BATTERY and #payload >= 8 then
    telemetry.volts = be16(payload, 1) / 10
    telemetry.amps = be16(payload, 3) / 10
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
  elseif kind == T_AIRSPEED and #payload >= 2 then
    telemetry.speed = (string.byte(payload, 1) * 256 + string.byte(payload, 2)) / 36
  elseif kind == T_VARIO and #payload >= 2 then
    telemetry.climb = be16(payload, 1) / 10
  elseif kind == T_ENVELOPE then
    -- MAVLink that the plane sent (to the radio's module): carry it on for the ground station.
    -- What the ground station sent (to the radio): carry it on to the plane.
    local to = nil
    if addr == ADDR_TX then to = ADDR_RADIO elseif addr == ADDR_RADIO then to = ADDR_FC end
    if to then
      serialWrite(frame(to, T_ENVELOPE, payload))
      telemetry.forwarded = telemetry.forwarded + 1
    end
  end
end

-- Cut the bytes that came down the cable into frames.
local function readLine()
  local got = serialRead(128)
  if got and #got > 0 then pending = pending .. got end
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
  buildCrc()
  return { zone = zone, options = opts }
end

local function update(widget, opts)
  widget.options = opts
end

local function background(widget)
  tick()
end

local function refresh(widget, event, touchState)
  tick()
  local z = widget.zone
  local live = getTime() - telemetry.last < 100
  lcd.drawText(z.x + 4, z.y + 2, "T1-S flight computer", MIDSIZE)
  lcd.drawText(z.x + 4, z.y + 28, live and (telemetry.mode .. (telemetry.armed and "  ARMED" or "  DISARMED")) or "NO TELEMETRY", MIDSIZE)
  lcd.drawText(z.x + 4, z.y + 56, string.format("BAT %.1f V  %.1f A  %d %%", telemetry.volts, telemetry.amps, telemetry.percent), 0)
  lcd.drawText(z.x + 4, z.y + 74, string.format("ROLL %d  PITCH %d", telemetry.roll, telemetry.pitch), 0)
  lcd.drawText(z.x + 4, z.y + 92, string.format("AS %.0f m/s  VS %.1f m/s", telemetry.speed, telemetry.climb), 0)
  lcd.drawText(z.x + 4, z.y + 110, string.format("LINK %d %%  GCS %d  FRAMES %d", telemetry.quality, telemetry.forwarded, telemetry.frames), 0)
end

return { name = "T1SFC", options = options, create = create, update = update, refresh = refresh, background = background }
