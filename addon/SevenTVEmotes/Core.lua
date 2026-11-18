-- SevenTVEmotes - Core
-- Replaces 7TV emote names in chat messages with in-game textures.
local ADDON_NAME, ns = ...

local floor, format = math.floor, string.format

-- Midnight-era clients (which WoW: Forever is built on) hand chat text to addons as
-- "secret values" inside instances. Addon code may not inspect those, so every entry
-- point checks first and leaves secret text untouched.
local issecret = issecretvalue or function() return false end
ns.issecret = issecret

local DEFAULTS = {
    enabled = true,       -- master switch
    size = 28,            -- emote height in chat (pixels)
    animate = true,       -- animate animated emotes
    fps = 30,             -- animation update rate
    links = true,         -- hover tooltip + shift-click
    autocomplete = true,  -- ":name" suggestions in the chat box
    minChars = 2,         -- characters after ":" before suggestions appear
    disabled = {},        -- [emoteName] = true
    queue = {},           -- requests for the helper: { id, kind, ref, alias, t }
    history = {},         -- finished requests, newest first: { ok, msg, name, kind, ref, t }
    compress = false,     -- ask the helper for compressed (BLP) textures
    minimap = true,       -- show the minimap button
    minimapAngle = 225,   -- its position around the minimap, degrees (0 = right, 90 = top)
}
ns.DEFAULTS = DEFAULTS

-- Working copy until SavedVariables load (also lets the logic run outside the game).
ns.db = {}
for k, v in pairs(DEFAULTS) do ns.db[k] = v end
ns.loadedCompress = false -- the compression setting as of this load (replaced on ADDON_LOADED)

---------------------------------------------------------------------------
-- Emote registry. Emotes come from "packs": the bundled Emotes.lua and the
-- helper-managed SevenTVEmotes_Local addon. Later packs override earlier ones.
---------------------------------------------------------------------------
ns.emotes = {}      -- name -> entry
ns.byId = {}        -- 7TV id -> entry
ns.animByPath = {}  -- texture path (no extension) -> entry, animated only
ns.names = {}       -- sorted names
ns.lnames = {}      -- lower-cased, same order as ns.names
ns.packs = {}
ns.count = 0
ns.hasAnimated = false

local emotes, byId = ns.emotes, ns.byId

local function Reindex()
    local names, lnames = ns.names, ns.lnames
    for i = #names, 1, -1 do names[i] = nil; lnames[i] = nil end
    for name in pairs(emotes) do names[#names + 1] = name end
    table.sort(names, function(a, b)
        local la, lb = a:lower(), b:lower()
        if la == lb then return a < b end
        return la < lb
    end)
    for i, name in ipairs(names) do lnames[i] = name:lower() end
    ns.count = #names
end

-- data[name] = { file, width, height, texWidth, texHeight, id [, frames, columns, msPerFrame] }
-- `file` is relative to the pack's addon folder, e.g. "slots\\s0001.blp" (".tga" assumed if no extension).
function ns.RegisterPack(packName, data, addonFolder)
    if type(data) ~= "table" then return end
    local root = "Interface\\AddOns\\" .. addonFolder .. "\\"
    local pack = { name = packName, count = 0, animated = 0 }
    ns.packs[#ns.packs + 1] = pack
    for name, d in pairs(data) do
        local file = d[1]
        if not file:find("%.%a+$") then file = file .. ".tga" end
        local e = {
            name = name, path = root .. file, w = d[2], h = d[3], tw = d[4], th = d[5],
            id = d[6] or d[1], pack = packName,
        }
        if d[7] and d[7] > 1 then
            e.frames, e.cols, e.ms = d[7], d[8], d[9]
            ns.animByPath[e.path] = e
            ns.hasAnimated = true
            pack.animated = pack.animated + 1
        end
        emotes[name] = e
        byId[e.id] = e
        pack.count = pack.count + 1
    end
    Reindex()
end

-- Public API for pack addons.
SevenTVEmotes = SevenTVEmotes or {}
SevenTVEmotes.RegisterPack = ns.RegisterPack

---------------------------------------------------------------------------
-- Texture strings
---------------------------------------------------------------------------
-- Pixel rect of frame f (0-based) inside the texture. Static emotes: f == nil.
function ns.Coords(e, f)
    if not f then return 0, e.w, 0, e.h end
    local l = (f % e.cols) * e.w
    local t = floor(f / e.cols) * e.h
    return l, l + e.w, t, t + e.h
end

-- Frame index for time `ms` (milliseconds). All copies of an emote stay in sync.
function ns.FrameAt(e, ms)
    return floor(ms / e.ms) % e.frames
end

local MAX_RATIO = 4 -- very wide emotes are shrunk so they fit within h*4

-- |Tpath:height:width:offX:offY:texW:texH:left:right:top:bottom|t
function ns.Tag(e, size, f)
    local h = size or ns.db.size
    local w = floor(h * e.w / e.h + 0.5)
    if w > h * MAX_RATIO then
        w = h * MAX_RATIO
        h = math.max(1, floor(w * e.h / e.w + 0.5))
    end
    local l, r, t, b = ns.Coords(e, f)
    return format("|T%s:%d:%d:0:0:%d:%d:%d:%d:%d:%d|t", e.path, h, w, e.tw, e.th, l, r, t, b)
end

function ns.Render(e)
    local f = e.frames and ns.FrameAt(e, GetTime() * 1000) or nil
    local tag = ns.Tag(e, nil, f)
    if ns.db.links then
        -- "addon" hyperlinks (10.1.0+) are local-only and safe to click.
        return format("|Haddon:%s:%s|h%s|h", ADDON_NAME, e.id, tag)
    end
    return tag
end

---------------------------------------------------------------------------
-- Message rewriting
---------------------------------------------------------------------------
-- Escape sequences that must pass through untouched (anchored at the "|").
local ESCAPES = {
    "^|H.-|h.-|h",            -- hyperlinks: items, players, quests, ...
    "^|T.-|t",                -- textures
    "^|A.-|a",                -- atlases
    "^|K.-|k",                -- protected strings (Battle.net names)
    "^|c%x%x%x%x%x%x%x%x",    -- color start
    "^|cn[^:|]*:",            -- named color start
    "^|r", "^|n", "^||",
}

local function FindEscape(s, init)
    local p = s:find("|", init, true)
    while p do
        for i = 1, #ESCAPES do
            local a, b = s:find(ESCAPES[i], p)
            if a then return a, b end
        end
        p = s:find("|", p + 1, true)
    end
end

local function ReplaceWord(word)
    local e = emotes[word]
    if e and not ns.db.disabled[word] then
        return ns.Render(e)
    end
end

local function EmotifyPlain(s)
    if s == "" then return s end
    return (s:gsub("%S+", ReplaceWord))
end

function ns.Emotify(msg)
    if issecret(msg) or type(msg) ~= "string" then return msg end
    if not msg:find("|", 1, true) then return EmotifyPlain(msg) end
    local out, n, pos = {}, 0, 1
    while true do
        local a, b = FindEscape(msg, pos)
        if not a then
            n = n + 1; out[n] = EmotifyPlain(msg:sub(pos))
            break
        end
        n = n + 1; out[n] = EmotifyPlain(msg:sub(pos, a - 1))
        n = n + 1; out[n] = msg:sub(a, b)
        pos = b + 1
    end
    return table.concat(out)
end

local function ChatFilter(_, _, msg, ...)
    if issecret(msg) or not ns.db.enabled or ns.count == 0 then return false end
    local new = ns.Emotify(msg)
    if new ~= msg then
        return false, new, ...
    end
    return false
end

local CHAT_EVENTS = {
    "CHAT_MSG_SAY", "CHAT_MSG_YELL", "CHAT_MSG_EMOTE",
    "CHAT_MSG_WHISPER", "CHAT_MSG_WHISPER_INFORM",
    "CHAT_MSG_BN_WHISPER", "CHAT_MSG_BN_WHISPER_INFORM",
    "CHAT_MSG_GUILD", "CHAT_MSG_OFFICER",
    "CHAT_MSG_PARTY", "CHAT_MSG_PARTY_LEADER",
    "CHAT_MSG_RAID", "CHAT_MSG_RAID_LEADER", "CHAT_MSG_RAID_WARNING",
    "CHAT_MSG_INSTANCE_CHAT", "CHAT_MSG_INSTANCE_CHAT_LEADER",
    "CHAT_MSG_CHANNEL", "CHAT_MSG_COMMUNITIES_CHANNEL",
    "CHAT_MSG_AFK", "CHAT_MSG_DND",
}

local function RegisterFilters()
    local add = ChatFrame_AddMessageEventFilter
        or (ChatFrameUtil and ChatFrameUtil.AddMessageEventFilter)
    if not add then
        ns.Print("Could not find the chat filter API; emotes will not show in chat.")
        return
    end
    for _, event in ipairs(CHAT_EVENTS) do
        add(event, ChatFilter)
    end
end

---------------------------------------------------------------------------
-- Chat frame tracking (shared with Animator and Autocomplete)
---------------------------------------------------------------------------
ns.chatFrames = {}
ns.chatFrameCallbacks = {}
local knownFrames = {}

local function AddChatFrame(cf)
    if not cf or knownFrames[cf] then return end
    knownFrames[cf] = true
    ns.chatFrames[#ns.chatFrames + 1] = cf
    for _, cb in ipairs(ns.chatFrameCallbacks) do cb(cf) end
end

function ns.RefreshChatFrames()
    if CHAT_FRAMES then
        for _, name in ipairs(CHAT_FRAMES) do AddChatFrame(_G[name]) end
    end
    for i = 1, (NUM_CHAT_WINDOWS or 10) do AddChatFrame(_G["ChatFrame" .. i]) end
end

function ns.OnChatFrame(cb)
    ns.chatFrameCallbacks[#ns.chatFrameCallbacks + 1] = cb
    for _, cf in ipairs(ns.chatFrames) do cb(cf) end
end

---------------------------------------------------------------------------
-- Hover tooltip and shift-click
---------------------------------------------------------------------------
local function ParseLink(link)
    if issecret(link) or type(link) ~= "string" then return end
    local kind, addon, id = strsplit(":", link)
    if kind == "addon" and addon == ADDON_NAME then return byId[id] end
end

local function OnHyperlinkEnter(cf, link)
    if not ns.db.links then return end
    local e = ParseLink(link)
    if not e then return end
    GameTooltip:SetOwner(cf, "ANCHOR_CURSOR")
    GameTooltip:AddLine(ns.Tag(e, 64, e.frames and 0 or nil))
    GameTooltip:AddLine(e.name, 1, 1, 1)
    GameTooltip:AddLine(e.frames and "7TV emote (animated)" or "7TV emote", 0.16, 0.71, 0.96)
    GameTooltip:AddLine("Shift-click while typing to add it to your message", 0.6, 0.6, 0.6)
    GameTooltip:Show()
    ns.tooltipEmote = e
    -- Rewriting a chat line's text (which the Animator does to animate emotes) makes the client
    -- fire OnHyperlinkLeave, hiding this tooltip a frame later. So the Animator leaves this chat
    -- frame alone while the tooltip is up; the tooltip's own copy of the emote keeps animating.
    ns.hoverFrame = cf
end

local function OnHyperlinkLeave()
    ns.hoverFrame = nil
    if ns.tooltipEmote then
        ns.tooltipEmote = nil
        GameTooltip:Hide()
    end
end

ns.OnHyperlinkEnter, ns.OnHyperlinkLeave = OnHyperlinkEnter, OnHyperlinkLeave -- (for tests)

local function GetActiveEditBox()
    if ChatEdit_GetActiveWindow then return ChatEdit_GetActiveWindow() end
    if ChatFrameUtil and ChatFrameUtil.GetActiveWindow then return ChatFrameUtil.GetActiveWindow() end
end

local function OnLinkClicked(link)
    local e = ParseLink(link)
    if not e or not IsShiftKeyDown() then return end
    local eb = GetActiveEditBox()
    if eb then
        eb:Insert(e.name .. " ")
    else
        ns.Print("Open the chat box first, then shift-click an emote to add it.")
    end
end

local function SetupLinks()
    ns.OnChatFrame(function(cf)
        cf:HookScript("OnHyperlinkEnter", OnHyperlinkEnter)
        cf:HookScript("OnHyperlinkLeave", OnHyperlinkLeave)
    end)
    if EventRegistry and EventRegistry.RegisterCallback then
        EventRegistry:RegisterCallback("SetItemRef", function(_, link) OnLinkClicked(link) end, ns)
    elseif SetItemRef then
        hooksecurefunc("SetItemRef", OnLinkClicked)
    end
end

---------------------------------------------------------------------------
-- Output
---------------------------------------------------------------------------
function ns.Print(msg)
    local cf = DEFAULT_CHAT_FRAME or ChatFrame1
    if cf then cf:AddMessage("|cff29b6f67TV Emotes|r: " .. tostring(msg)) end
end

function ns.OnOff(v)
    return v and "|cff40ff40on|r" or "|cffff4040off|r"
end

---------------------------------------------------------------------------
-- Settings panel
---------------------------------------------------------------------------
local function SetupSettings()
    if not (Settings and Settings.RegisterVerticalLayoutCategory and Settings.RegisterAddOnSetting) then return end
    local ok, err = pcall(function()
        local category = Settings.RegisterVerticalLayoutCategory("7TV Emotes")
        local VT = Settings.VarType or {}
        local function setting(key, label)
            local vt = type(DEFAULTS[key]) == "boolean" and (VT.Boolean or "boolean") or (VT.Number or "number")
            return Settings.RegisterAddOnSetting(category, "SevenTVEmotes_" .. key, key, ns.db, vt, label, DEFAULTS[key])
        end
        local function checkbox(key, label, tip)
            Settings.CreateCheckbox(category, setting(key, label), tip)
        end
        checkbox("enabled", "Show emotes in chat", "Replace 7TV emote names in chat with images.")
        checkbox("animate", "Animate emotes", "Play animated emotes. Turn off if chat feels slow.")
        checkbox("autocomplete", "Autocomplete", "Type : followed by part of a name to get suggestions. Tab cycles, a second : accepts.")
        checkbox("links", "Hover tooltips", "Show the emote name when hovering it; shift-click to reuse it.")
        local options = Settings.CreateSliderOptions(12, 64, 1)
        if MinimalSliderWithSteppersMixin and options.SetLabelFormatter then
            options:SetLabelFormatter(MinimalSliderWithSteppersMixin.Label.Right)
        end
        Settings.CreateSlider(category, setting("size", "Emote size"), options, "Height of emotes in chat, in pixels. Applies to new messages.")
        Settings.RegisterAddOnCategory(category)
        ns.settingsCategory = category
    end)
    if not ok then ns.settingsError = err end
end

function ns.OpenSettings()
    if ns.settingsCategory and Settings and Settings.OpenToCategory then
        Settings.OpenToCategory(ns.settingsCategory:GetID())
    else
        ns.Print("Settings panel unavailable on this client; use the /7tv commands.")
    end
end

---------------------------------------------------------------------------
-- Startup
---------------------------------------------------------------------------
-- Pack shipped inside this addon (Emotes.lua); empty unless emotes are baked in for a release.
if ns.EmoteData and next(ns.EmoteData) then
    ns.RegisterPack("Bundled", ns.EmoteData, ADDON_NAME)
end

local events = CreateFrame("Frame")
events:RegisterEvent("ADDON_LOADED")
events:RegisterEvent("PLAYER_LOGIN")
events:SetScript("OnEvent", function(self, event, arg1)
    if event == "ADDON_LOADED" and arg1 == ADDON_NAME then
        SevenTVEmotesDB = SevenTVEmotesDB or {}
        local db = SevenTVEmotesDB
        for k, v in pairs(DEFAULTS) do
            if db[k] == nil then db[k] = (type(v) == "table") and {} or v end
        end
        ns.db = db
        ns.loadedCompress = db.compress and true or false
        RegisterFilters()
        SetupSettings()
        self:UnregisterEvent("ADDON_LOADED")
    elseif event == "PLAYER_LOGIN" then
        ns.RefreshChatFrames()
        if FCF_OpenTemporaryWindow then
            hooksecurefunc("FCF_OpenTemporaryWindow", function()
                C_Timer.After(0, ns.RefreshChatFrames)
            end)
        end
        SetupLinks()
        if ns.SetupAutocomplete then ns.SetupAutocomplete() end
        if ns.OnLoginHelper then ns.OnLoginHelper() end
    end
end)
