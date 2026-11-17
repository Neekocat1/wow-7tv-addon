-- SevenTVEmotes - Animator
-- Animated emotes are sprite sheets. Chat lines are static text, so on a timer we
-- rewrite the texture coordinates of animated emotes in the visible chat lines.
local ADDON_NAME, ns = ...

local floor, format = math.floor, string.format
local issecret = ns.issecret

-- Any texture from this addon family: Interface\AddOns\SevenTVEmotes[_Pack]\...\file.tga
local PLAIN = "|TInterface\\AddOns\\SevenTVEmotes"
local PATTERN = "|T(Interface\\AddOns\\SevenTVEmotes[^:|]*%.%a+):(%d+):(%d+):[^|]*|t"

-- Textures (e.g. autocomplete icons) that should animate: [texture] = emote entry.
ns.animTextures = setmetatable({}, { __mode = "k" })

local now = 0

local function SwapFrame(path, h, w)
    local e = ns.animByPath[path]
    if not e then return nil end -- static emote: leave as is
    local l, r, t, b = ns.Coords(e, ns.FrameAt(e, now))
    return format("|T%s:%s:%s:0:0:%d:%d:%d:%d:%d:%d|t", path, h, w, e.tw, e.th, l, r, t, b)
end

-- Exposed for tests: returns the text with every animated emote advanced to `ms`.
function ns.AnimateText(text, ms)
    now = ms
    return (text:gsub(PATTERN, SwapFrame))
end

local function AnimateFontString(fs)
    if not fs or not fs:IsShown() then return end
    local text = fs:GetText()
    if issecret(text) or not text or not text:find(PLAIN, 1, true) then return end
    local new = text:gsub(PATTERN, SwapFrame)
    if new ~= text then fs:SetText(new) end
end

local function AnimateChatFrame(cf)
    if not cf:IsVisible() then return end
    local lines = cf.visibleLines
    if lines then
        for i = 1, #lines do AnimateFontString(lines[i]) end
    elseif cf.FontStringContainer then
        -- Fallback if the client's ScrollingMessageFrame layout differs.
        for _, region in ipairs({ cf.FontStringContainer:GetRegions() }) do
            if region.GetText then AnimateFontString(region) end
        end
    end
end

local driver = CreateFrame("Frame")
local elapsedSince = 0
driver:SetScript("OnUpdate", function(_, elapsed)
    local db = ns.db
    if not (db.enabled and db.animate and ns.hasAnimated) then return end
    elapsedSince = elapsedSince + elapsed
    if elapsedSince < 1 / (db.fps or 30) then return end
    elapsedSince = 0
    now = GetTime() * 1000

    local frames = ns.chatFrames
    for i = 1, #frames do
        pcall(AnimateChatFrame, frames[i])
    end

    if ns.tooltipEmote and GameTooltip:IsVisible() and GameTooltipTextLeft1 then
        pcall(AnimateFontString, GameTooltipTextLeft1)
    end

    for tex, e in pairs(ns.animTextures) do
        if tex:IsVisible() then
            local l, r, t, b = ns.Coords(e, ns.FrameAt(e, now))
            tex:SetTexCoord(l / e.tw, r / e.tw, t / e.th, b / e.th)
        end
    end
end)
