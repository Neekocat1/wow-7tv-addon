-- Run: texlua tests/test_addon.lua <path to generated Local.lua>
local LOCAL = arg[1] or "/tmp/Local.lua"
local NOW = 1794701336 + 60
function time() return NOW end
function GetTime() return 1.0 end
function wipe(t) for k in pairs(t) do t[k] = nil end return t end
function strsplit(sep, s) local out = {} for p in (s..sep):gmatch("(.-)"..sep:gsub("%p","%%%0")) do out[#out+1]=p end return table.unpack(out) end
local function dummy() local o; o = setmetatable({}, {__index=function(_,k) return function() return o end end}); return o end
function CreateFrame() return dummy() end
UIParent = dummy(); SlashCmdList = {}
local printed = {}
DEFAULT_CHAT_FRAME = { AddMessage = function(_, m) printed[#printed+1] = m end }
function hooksecurefunc() end
local timers = {}
C_Timer = { After = function(d, fn) timers[#timers+1] = fn end }
StaticPopupDialogs = {}
local popups = {}
function StaticPopup_Show(which, text) popups[#popups+1] = { which = which, text = text } end
IsShiftKeyDown = function() return false end

local ns = {}
for _, f in ipairs({"Emotes.lua","Core.lua","Animator.lua","Autocomplete.lua","Helper.lua","UI.lua","LiveStatus.lua","Commands.lua"}) do
  assert(loadfile("addon/SevenTVEmotes/"..f))("SevenTVEmotes", ns)
end
-- Saved variables as WoW would load them, with pending requests r1..r3 and one unknown
ns.db.queue = { {id="r1",kind="add",ref="x"}, {id="r7",kind="channel",ref="nobody"}, {id="zzz",kind="add",ref="later"} }
assert(loadfile(LOCAL))()   -- the helper's file (own addon, globals only)
NOW = SevenTVEmotes_Helper.heartbeat + 60

local function check(c, msg) if not c then error("FAIL: "..msg, 2) end print("pass: "..msg) end
check(ns.count >= 6, "emotes registered from Local.lua: "..ns.count)
check(ns.emotes.catJAM.path == "Interface\\AddOns\\SevenTVEmotes_Local\\slots\\s0001.tga", "slot path")
ns.db.links = false
local t = ns.Emotify("hi catJAM Clap")
print(t)
check(t:find("|TInterface\\AddOns\\SevenTVEmotes_Local\\slots\\s0002.tga:28:28:0:0:64:64:0:64:0:64|t", 1, true), "static slot tag")
local a = ns.AnimateText(t, 50 * 7)  -- frame 7: col 1, row 2
print(a)
check(a:find("s0001.tga:28:28:0:0:256:512:48:96:96:144|t", 1, true), "animated slot tag advanced")

ns.OnLoginHelper()
for _, m in ipairs(printed) do print("  chat: "..m) end
check(#ns.db.queue == 1 and ns.db.queue[1].id == "zzz", "processed requests removed from queue")
check(printed[1]:find("Added WideDank") and printed[1]:find("|T"), "success printed with icon")
check(printed[2]:find("nobody"), "failure printed")
check(#timers == 1, "status bar refresh scheduled after login")
check(#popups == 0, "no popups at login")

-- stale helper
NOW = NOW + 3600
check(ns.HelperState() == "stale", "stale detection")
NOW = NOW - 3600

-- slash add queues + popup
SlashCmdList.SEVENTVEMOTES("add https://7tv.app/emotes/01F6MQ33FG000FFJ97ZB8MWV52 myAlias")
check(#ns.db.queue == 2 and ns.db.queue[2].alias == "myAlias" and ns.db.queue[2].kind == "add", "add queued")
check(#popups == 0 and printed[#printed]:find("Click Send on the 7TV bar"), "no popup; chat hint instead")
SlashCmdList.SEVENTVEMOTES("channel @Tester")
check(ns.db.queue[3].kind == "channel" and ns.db.queue[3].ref == "tester", "channel queued lowercased")
SlashCmdList.SEVENTVEMOTES("remove NotAnEmote")
check(#ns.db.queue == 3, "remove unknown rejected")
SlashCmdList.SEVENTVEMOTES("helper")
print(printed[#printed-1]); print(printed[#printed])
-- missing helper
SevenTVEmotes_Helper = nil
local n = #ns.db.queue
SlashCmdList.SEVENTVEMOTES("add catJAM")
check(#ns.db.queue == n and printed[#printed]:find("Helper app"), "add without helper explains")
print("ALL GOOD")
