-- Exercises the /7tv window with mock frames that keep state.
-- Run: texlua tests/test_ui.lua <path to generated Local.lua>
local LOCAL = arg[1] or "/tmp/Local.lua"
local NOW = 1794703534 + 120
function time() return NOW end
function GetTime() return 1.0 end
function GetCursorPosition() return 0, 0 end
function wipe(t) for k in pairs(t) do t[k] = nil end return t end
function strsplit(sep, s) local out = {} for p in (s..sep):gmatch("(.-)"..sep:gsub("%p","%%%0")) do out[#out+1]=p end return table.unpack(out) end
unpack = table.unpack

-- Mock widgets ---------------------------------------------------------------
local Mock = {}
Mock.__index = function(t, k)
    local v = rawget(Mock, k)
    if v then return v end
    if k:match("^%u") then return function() end end -- any other API method is a no-op
    return nil                                       -- plain fields behave like real frames
end
local function new(kind, parent)
    local o = setmetatable({ kind = kind, shown = true, parent = parent, scripts = {}, w = 0, h = 0, enabled = true }, Mock)
    return o
end
function Mock:Show() self.shown = true; if self.scripts.OnShow then self.scripts.OnShow(self) end end
function Mock:Hide() local was = self.shown; self.shown = false; if was and self.scripts.OnHide then self.scripts.OnHide(self) end end
function Mock:SetShown(v) if v then self:Show() else self:Hide() end end
function Mock:IsShown() return self.shown end
function Mock:IsVisible()
    local o = self
    while o do if not o.shown then return false end o = o.parent end
    return true
end
function Mock:SetText(t) self.text = t; if self.scripts.OnTextChanged and self.kind == "EditBox" then self.scripts.OnTextChanged(self, false) end end
function Mock:GetText() return self.text or "" end
function Mock:SetScript(n, f) self.scripts[n] = f end
function Mock:GetScript(n) return self.scripts[n] end
function Mock:HookScript(n, f) self.scripts[n] = f end
function Mock:CreateTexture() return new("Texture", self) end
function Mock:CreateFontString() return new("FontString", self) end
function Mock:SetSize(w, h) self.w, self.h = w, h end
function Mock:SetWidth(w) self.w = w end
function Mock:SetHeight(h) self.h = h end
function Mock:GetHeight() return self.h end
function Mock:GetWidth() return self.w end
function Mock:Enable() self.enabled = true end
function Mock:Disable() self.enabled = false end
function Mock:SetEnabled(v) self.enabled = v and true or false end
function Mock:IsEnabled() return self.enabled end
function Mock:SetTexture(p) self.texture = p end
function Mock:SetTexCoord(...) self.coords = { ... } end
function Mock:GetEffectiveScale() return 1 end
function Mock:GetPoint() return "CENTER", nil, "CENTER", 0, 0 end
function Mock:GetTop() return 0 end
function Mock:Click() if self.scripts.OnClick then self.scripts.OnClick(self, "LeftButton") end end

function CreateFrame(kind, name, parent)
    local f = new(kind, parent)
    if name then _G[name] = f end
    return f
end
UIParent = new("Frame")
GameTooltip = new("GameTooltip")
UISpecialFrames = {}
SlashCmdList = {}
local printed = {}
DEFAULT_CHAT_FRAME = { AddMessage = function(_, m) printed[#printed + 1] = m end }
function hooksecurefunc() end
C_Timer = { After = function() end }
StaticPopupDialogs = {}
function StaticPopup_Show() end
IsShiftKeyDown = function() return false end
local reloaded = false
C_UI = { Reload = function() reloaded = true end }
Minimap = new("Frame")
Minimap.w, Minimap.h = 140, 140
function Minimap:GetCenter() return 500, 400 end
local cursorX, cursorY = 0, 0
function GetCursorPosition() return cursorX, cursorY end
local tipLines = {}
GameTooltip.AddLine = function(_, t) tipLines[#tipLines + 1] = t end

local ns = {}
for _, f in ipairs({ "Emotes.lua", "Core.lua", "Animator.lua", "Autocomplete.lua", "Helper.lua", "UI.lua", "LiveStatus.lua", "Minimap.lua", "Commands.lua" }) do
    assert(loadfile("addon/SevenTVEmotes/" .. f))("SevenTVEmotes", ns)
end
assert(loadfile(LOCAL))()
NOW = SevenTVEmotes_Helper.heartbeat + 120
ns.db.queue = { { id = "old1", kind = "add", ref = "https://7tv.app/emotes/X", t = NOW - 300 } }

local function check(c, msg) if not c then error("FAIL: " .. msg, 2) end print("pass: " .. msg) end
local UI = ns.UI

-- open with /7tv ----------------------------------------------------------------
SlashCmdList.SEVENTVEMOTES("")
local f = SevenTVEmotesFrame
check(f and f:IsShown(), "/7tv opens the window")
check(UI.summary.text:find("6 emotes") and UI.summary.text:find("MB on disk"), "header shows count and disk use: " .. UI.summary.text)
check(UI.footerText.text:find("working on 1 request"), "footer shows sent request: " .. UI.footerText.text)
check(UI.downloadsTab.label.text == "Downloads (1)", "downloads tab badge")

-- Emotes tab --------------------------------------------------------------------
local ep = UI.pages.emotes.ctl
check(#ep.list == 6 and ep.cells[1].shown and ep.cells[1].emote, "grid filled")
local animCell
for _, c in ipairs(ep.cells) do if c.emote and c.emote.frames then animCell = c end end
check(animCell and ns.animTextures[animCell.icon], "animated cell registered with animator")
check(animCell.icon.texture:find("SevenTVEmotes_Local\\slots\\s0001.tga", 1, true), "cell texture path")
ep.search:SetText("pee"); ep.search.scripts.OnTextChanged(ep.search, true)
check(#ep.list == 1 and ep.cells[1].emote.name == "peepoHappy", "search filters")
ep.cells[1]:Click()
check(ep.detail.name.text == "peepoHappy" and ep.detail.info.text:find("Added by you"), "detail shows origin: " .. ep.detail.info.text)
check(ep.detail.remove.enabled, "remove enabled for my emote")
ep.detail.hide:Click()
check(ns.db.disabled.peepoHappy and ep.detail.hide.label.text == "Show in chat", "hide toggles")
check(ns.Emotify("peepoHappy") == "peepoHappy", "hidden emote stays text")
ep.detail.hide:Click()
ep.detail.remove:Click()
check(ns.db.queue[#ns.db.queue].kind == "remove" and ep.detail.remove.label.text == "Removal queued", "remove queues request")
check(UI.footerText.text:find("1 request waiting to be sent") and UI.reload.label.text == "Send to helper", "footer offers Send")
local bar = SevenTVEmotesStatusBar
check(bar and bar.shown and bar.button.label.text == "Send", "status bar appears with Send (sent + unsent)")
check(bar.live.active and bar.live.front.texture and bar.live.front.texture:find("SevenTVEmotes_Local\\status\\st%d%d%d%d%.tga"), "live board image path: " .. tostring(bar.live.front.texture))
check(bar.live.back.texture and bar.live.back.texture ~= bar.live.front.texture, "next image preloaded in the back buffer")
ep.search:SetText(""); ep.search.scripts.OnTextChanged(ep.search, true)
UI.pages.emotes.ctl.filterButtons[2]:Click() -- Mine
check(#ep.list == 6, "mine filter (all are local here)")
UI.pages.emotes.ctl.filterButtons[3]:Click() -- Animated
check(#ep.list == 2, "animated filter")

-- Add tab -----------------------------------------------------------------------
UI.tabs.add:Click()
local ap = UI.pages.add.ctl
check(UI.pages.add.frame.shown and not UI.pages.emotes.frame.shown, "tab switch")
ap.link:SetText("https://7tv.app/emotes/01F6MQ33FG000FFJ97ZB8MWV52"); ap.alias:SetText("myCat")
ap.addBtn:Click()
local last = ns.db.queue[#ns.db.queue]
check(last.kind == "add" and last.alias == "myCat" and ap.link.text == "", "add emote queues and clears form")
check(ap.feedback.text:find("queued"), "add feedback: " .. ap.feedback.text)
ap.channel:SetText("@SomeStreamer"); ap.chanBtn:Click()
check(ns.db.queue[#ns.db.queue].ref == "somestreamer", "channel queued")
check(ap.chanRows[1].text.text == "somestreamer" and ap.chanRows[1].status.text:find("queued"), "queued channel listed")
check(ap.chanRows[2].text.text == "tester", "helper's channel listed")
ap.chanRows[2].btn:Click()
check(ns.db.queue[#ns.db.queue].kind == "unchannel", "remove channel queues")
ap.channel:SetText("somestreamer"); ap.chanBtn:Click()
check(ap.feedback.text:find("already queued"), "duplicate rejected")

-- Downloads tab -----------------------------------------------------------------
UI.tabs.downloads:Click()
local dp = UI.pages.downloads.ctl
check(dp.title.text == "Helper app running", "helper status")
check(dp.rows[1].state.text:find("In progress") and dp.rows[1].detail.text:find("5 min ago"), "sent request in progress")
check(dp.rows[2].state.text:find("Not sent yet"), "new request not sent")
check(UI.downloadsTab.label.text:find("%(5%)"), "badge counts all: " .. UI.downloadsTab.label.text)
dp.rows[2].btn:Click()
check(UI.downloadsTab.label.text:find("%(4%)"), "cancel removes request")
-- history appears after the helper reports back
ns.db.history = { { ok = true, msg = "Added catJAM.", name = "catJAM", t = NOW - 60 }, { ok = false, msg = "Nope.", t = NOW - 30 } }
UI:Refresh()
local found
for i = 1, #dp.items do if dp.items[i].title == "Added catJAM." then found = dp.items[i] end end
check(found and found.state:find("Done"), "history shown")
-- retrying & waiting from helper status
SevenTVEmotes_Helper.retrying = { old1 = "7TV couldn't be reached. Retrying (attempt 1 of 3)." }
SevenTVEmotes_Helper.waiting = { "bigEmote" }
UI:Refresh()
check(dp.rows[1].state.text:find("Retrying"), "retrying status")
local waitRow
for _, it in ipairs(dp.items) do if it.title:find("after you restart") then waitRow = it end end
check(waitRow and waitRow.detail == "bigEmote", "needs-restart row")

-- progress, helper problems and invisible files
SevenTVEmotes_Helper.progress = { done = 37, total = 412, t = NOW - 10 }
SevenTVEmotes_Helper.notice = "Couldn't download 7TV's global emotes yet (offline)."
local realSet = Mock.SetTexture
Mock.SetTexture = function(self, p) self.texture = p; if p:find("s0002", 1, true) then return false end return true end
check(ns.ProbeTextures() == 1, "probe finds a file WoW can't see")
Mock.SetTexture = realSet
UI:Refresh()
local seen = {}
for _, it in ipairs(dp.items) do seen[#seen + 1] = it.title end
local all = table.concat(seen, " | ")
check(all:find("37 of 412 emotes downloaded"), "progress row: " .. all)
check(all:find("global emotes yet"), "helper problem row")
check(all:find("1 emote can't be shown until you restart WoW"), "needs-restart row from probe")
check(#ns.db.invisible == 1, "invisible files reported to helper")

-- /7tv debug runs
SlashCmdList.SEVENTVEMOTES("debug")
check(printed[#printed]:find("Helper says") or printed[#printed]:find("Looks like"), "debug output")

-- Settings tab ------------------------------------------------------------------
UI.tabs.settings:Click()
local sp = UI.pages.settings.ctl
check(sp.storage.text:find("on disk") and sp.storage.text:find("uncompressed"), "storage line: " .. sp.storage.text)
check(sp.compressNote.text:find("bring this down"), "compression estimate: " .. sp.compressNote.text)
sp.compress:Click()
check(ns.db.compress == true and sp.compressNote.text:find("Click Send to apply"), "compress toggle pending: " .. sp.compressNote.text)
check(UI.footerText.text:find("compression setting changed"), "footer mentions compression")
sp.plus:Click()
check(ns.db.size == 30 and sp.sizeLabel.text == "Emote size: 30", "size +")
UI.reload:Click()
check(reloaded, "reload button reloads")

-- status bar: live image advances with the clock, without a reload ------------------
ns.db.queue = { { id = "s1", kind = "add", ref = "x", t = NOW } }   -- sent (not this session)
ns.sessionIds = {}
ns.db.compress = ns.loadedCompress
UI:Refresh()
bar = SevenTVEmotesStatusBar
check(bar.shown and bar.button.label.text == "Reload", "live mode with Reload when only sent requests")
local p1, preloaded = bar.live.front.texture, bar.live.back.texture
local frontTex = bar.live.front
NOW = NOW + 3
bar.live:Tick()
check(bar.live.front.texture == preloaded and bar.live.front ~= frontTex, "swapped to the preloaded image (no reload, no flicker)")
check(bar.live.back.texture ~= preloaded and bar.live.back.texture ~= p1, "following image preloading: " .. bar.live.back.texture)
bar.close:Click()
check(not bar.shown, "x hides the bar")
UI:Refresh()
check(not bar.shown, "stays hidden after dismiss")
SlashCmdList.SEVENTVEMOTES("status")
check(bar.shown, "/7tv status shows it again")
bar.close:Click()
ns.QueueRequest("add", "https://7tv.app/emotes/ZZZ", nil, "Emote", true)
check(bar.shown and bar.button.label.text == "Send", "new request re-shows the bar")
ns.db.queue = {}
ns.sessionIds = {}
Bar = ns.StatusBar; Bar.manual = false; Bar.dismissed = false
UI:Refresh()
check(bar.shown, "bar stays while the helper reported download progress")
SevenTVEmotes_Helper.progress = nil
UI:Refresh()
check(not bar.shown, "bar hides when nothing is pending")

-- texture test window
SlashCmdList.SEVENTVEMOTES("texturetest")
check(SevenTVEmotesTextureTest and SevenTVEmotesTextureTest.shown and SevenTVEmotesTextureTest.rows[2].texture:find("test\\t_dxt5m.blp", 1, true), "texture test window")

-- close / toggle ------------------------------------------------------------------
SlashCmdList.SEVENTVEMOTES("")
check(not f:IsShown(), "/7tv toggles closed")
SlashCmdList.SEVENTVEMOTES("downloads")
check(f:IsShown() and UI.current == "downloads", "/7tv downloads opens tab")

-- missing helper ------------------------------------------------------------------
SevenTVEmotes_Helper = nil
UI.tabs.add:Click()
check(not ap.addBtn.enabled and ap.banner.text:find("isn't installed"), "add disabled without helper")

-- minimap button --------------------------------------------------------------------
local MM = ns.Minimap
local mb = MM:Create()
check(mb and SevenTVEmotesMinimapButton == mb and mb.parent == Minimap and mb.shown, "minimap button created on the minimap")
check(mb.icon.texture == "Interface\\AddOns\\SevenTVEmotes\\media\\minimap", "minimap button icon")
f:Hide()
mb:Click()
check(f:IsShown() and UI.current ~= "settings", "left-click opens the window")
mb:Click()
check(not f:IsShown(), "left-click again closes it")
mb.scripts.OnClick(mb, "RightButton")
check(f:IsShown() and UI.current == "settings", "right-click opens settings")
-- position: 225 degrees (bottom-left) by default; dragging follows the cursor around the centre
local x, y = MM.Offset(225, 75, "ROUND")
check(math.abs(x + 53.03) < 0.1 and math.abs(y + 53.03) < 0.1, "default position bottom-left of the minimap")
x, y = MM.Offset(45, 75, "SQUARE")
check(x > 53 and y > 53 and x < 75 * 1.3, "square minimaps push the button to the edge")
cursorX, cursorY = 500, 600  -- straight above the minimap centre
mb.scripts.OnDragStart(mb)
mb.scripts.OnUpdate(mb)
mb.scripts.OnDragStop(mb)
check(math.abs(ns.db.minimapAngle - 90) < 0.01 and not mb.scripts.OnUpdate, "dragging moves the button and saves the angle")
-- tooltip
tipLines = {}
mb.scripts.OnEnter(mb)
check(tipLines[1]:find("Emotes") and tipLines[2]:find("emotes") and table.concat(tipLines, "\n"):find("Left%-click"), "minimap tooltip")
-- hide / show
SlashCmdList.SEVENTVEMOTES("minimap off")
check(not mb.shown and ns.db.minimap == false, "/7tv minimap off hides it")
SlashCmdList.SEVENTVEMOTES("minimap")
check(mb.shown and ns.db.minimap == true, "/7tv minimap toggles it back")
UI:Show("settings")
local mmToggle
for _, t in ipairs(UI.pages.settings.ctl.toggles) do if t.text.text == "Minimap button" then mmToggle = t end end
check(mmToggle and mmToggle.check.shown, "settings has a minimap toggle (on)")
mmToggle:Click()
check(not mb.shown and not mmToggle.check.shown, "settings toggle hides the button")
mmToggle:Click()
check(mb.shown, "settings toggle shows it again")
-- addon compartment entry
f:Hide()
SevenTVEmotes_OnAddonCompartmentClick(nil, "LeftButton")
check(f:IsShown(), "addon compartment click opens the window")
print("ALL GOOD")
