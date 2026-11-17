-- SevenTVEmotes - live status bar
-- A small bar above the chat box that appears while something is waiting:
--   queued mode : requests not sent yet -> "Send" button (reloads; logging out also sends them)
--   live mode   : the helper is working -> shows the helper's live status image + "Reload"
--
-- The live image is a texture file the helper keeps rewriting (see status_board.py). WoW reads a
-- texture's contents the first time it's displayed, so showing a different file every few
-- seconds gives an up-to-date picture without a reload. The addon can't read the picture; it's
-- for the player, who can then reload at the right moment.
local ADDON_NAME, ns = ...
local floor, format = math.floor, string.format

local Bar = {}
ns.StatusBar = Bar

local function Board()
    local H = SevenTVEmotes_Helper
    if type(H) == "table" and type(H.board) == "table" and H.board.count and H.board.period then
        return H.board
    end
end
ns.Board = Board

function ns.BoardIndex(t)
    local b = Board()
    if not b then return nil end
    return floor((t or time()) / b.period) % b.count
end

function ns.BoardPath(k)
    local b = Board()
    if not b or not k then return nil end
    return format("Interface\\AddOns\\SevenTVEmotes_Local\\%s\\st%04d.tga", b.dir or "status", k)
end

-- A live board image. Swapping a texture's file makes WoW load the new image in the background,
-- which shows as a flicker. So each live image is two stacked textures: the next file is loaded
-- into the hidden one ahead of time, and they swap the moment the period ticks over.
local liveImages = setmetatable({}, { __mode = "k" })

local LiveImage = {}
LiveImage.__index = LiveImage

function ns.CreateLiveImage(parent, w, h)
    local img = setmetatable({ parent = parent, layer = "ARTWORK" }, LiveImage)
    img.front = parent:CreateTexture(nil, "ARTWORK", nil, 2)
    img.back = parent:CreateTexture(nil, "ARTWORK", nil, 1)
    img.front:SetSize(w, h)
    img.back:SetSize(w, h)
    img.back:SetAlpha(0.01) -- practically invisible, but still loaded and drawn
    liveImages[img] = true
    return img
end

function LiveImage:SetPoint(...)
    self.front:SetPoint(...)
    self.back:SetPoint(...)
end

function LiveImage:Show()
    self.front:Show()
    self.back:Show()
    self.active = true
    self:Tick()
end

function LiveImage:Hide()
    self.front:Hide()
    self.back:Hide()
    self.active = false
end

function LiveImage:IsVisible()
    return self.active and self.parent:IsVisible()
end

function LiveImage:Tick()
    local b = Board()
    if not b then return end
    local k = floor(time() / b.period) % b.count
    local nextK = (k + 1) % b.count
    if self.frontK ~= k then
        if self.backK == k then
            -- Preloaded: swap instantly.
            self.front, self.back = self.back, self.front
            self.front:SetDrawLayer("ARTWORK", 2)
            self.back:SetDrawLayer("ARTWORK", 1)
            self.front:SetAlpha(1)
            self.back:SetAlpha(0.01)
            self.frontK, self.backK = k, self.frontK
        else
            self.front:SetTexture(ns.BoardPath(k))
            self.front:SetAlpha(1)
            self.frontK = k
        end
    end
    if self.backK ~= nextK then
        self.back:SetTexture(ns.BoardPath(nextK))
        self.back:SetAlpha(0.01)
        self.backK = nextK
    end
end

local driver = CreateFrame("Frame")
local acc = 0
driver:SetScript("OnUpdate", function(_, elapsed)
    acc = acc + elapsed
    if acc < 0.25 then return end
    acc = 0
    for img in pairs(liveImages) do
        if img:IsVisible() then img:Tick() end
    end
end)

---------------------------------------------------------------------------
-- The bar
---------------------------------------------------------------------------
local ACCENT = { 0.16, 0.71, 0.96 }

local function SmallButton(parent, label, w, h, onClick)
    local b = CreateFrame("Button", nil, parent, "BackdropTemplate")
    b:SetSize(w, h)
    b:SetBackdrop({ bgFile = "Interface\\Buttons\\WHITE8X8", edgeFile = "Interface\\Buttons\\WHITE8X8", edgeSize = 1 })
    b:SetBackdropColor(ACCENT[1] * 0.45, ACCENT[2] * 0.45, ACCENT[3] * 0.45, 1)
    b:SetBackdropBorderColor(0.25, 0.25, 0.3, 1)
    b.label = b:CreateFontString(nil, "OVERLAY", "GameFontHighlightSmall")
    b.label:SetPoint("CENTER")
    b.label:SetText(label)
    b:SetScript("OnClick", onClick)
    b:SetScript("OnEnter", function(s)
        s:SetBackdropBorderColor(ACCENT[1], ACCENT[2], ACCENT[3], 1)
        if s.tip then
            GameTooltip:SetOwner(s, "ANCHOR_TOP")
            GameTooltip:AddLine(s.tip, 1, 1, 1, true)
            GameTooltip:Show()
        end
    end)
    b:SetScript("OnLeave", function(s)
        s:SetBackdropBorderColor(0.25, 0.25, 0.3, 1)
        if s.tip then GameTooltip:Hide() end
    end)
    return b
end

function Bar:Create()
    if self.frame then return end
    local f = CreateFrame("Frame", "SevenTVEmotesStatusBar", UIParent, "BackdropTemplate")
    self.frame = f
    f:SetSize(352, 44)
    f:SetFrameStrata("MEDIUM")
    f:SetClampedToScreen(true)
    f:EnableMouse(true)
    f:SetMovable(true)
    f:RegisterForDrag("LeftButton")
    f:SetScript("OnDragStart", f.StartMoving)
    f:SetScript("OnDragStop", function(s)
        s:StopMovingOrSizing()
        local p1, _, p2, x, y = s:GetPoint()
        ns.db.barPos = { p1, p2, floor(x + 0.5), floor(y + 0.5) }
    end)
    f:SetBackdrop({ bgFile = "Interface\\Buttons\\WHITE8X8", edgeFile = "Interface\\Buttons\\WHITE8X8", edgeSize = 1 })
    f:SetBackdropColor(0.05, 0.05, 0.07, 0.92)
    f:SetBackdropBorderColor(ACCENT[1], ACCENT[2], ACCENT[3], 0.6)

    -- Fallback text sits underneath the live image; it shows through if the image is missing.
    f.fallback = f:CreateFontString(nil, "BACKGROUND", "GameFontDisableSmall")
    f.fallback:SetPoint("LEFT", 12, 0)
    f.fallback:SetWidth(250)
    f.fallback:SetJustifyH("LEFT")
    f.live = ns.CreateLiveImage(f, 256, 32)
    f.live:SetPoint("LEFT", 6, 0)
    f.title = f:CreateFontString(nil, "OVERLAY", "GameFontHighlight")
    f.title:SetPoint("TOPLEFT", 12, -7)
    f.title:SetWidth(250)
    f.title:SetJustifyH("LEFT")
    f.sub = f:CreateFontString(nil, "OVERLAY", "GameFontDisableSmall")
    f.sub:SetPoint("TOPLEFT", f.title, "BOTTOMLEFT", 0, -2)
    f.sub:SetWidth(250)
    f.sub:SetJustifyH("LEFT")

    f.button = SmallButton(f, "Reload", 60, 24, function() ns.ReloadUI() end)
    f.button:SetPoint("RIGHT", -24, 0)
    f.close = SmallButton(f, "x", 16, 16, function()
        Bar.dismissed, Bar.manual = true, false
        f:Hide()
    end)
    f.close:SetBackdropColor(0.13, 0.13, 0.16, 1)
    f.close:SetPoint("TOPRIGHT", -3, -3)
    f.close.tip = "Hide until something new is queued. /7tv status shows it again."
    f:SetScript("OnMouseUp", function(_, btn) if btn == "RightButton" then ns.UI:Show("downloads") end end)

    local pos = ns.db.barPos
    if type(pos) == "table" and pos[1] then
        f:SetPoint(pos[1], UIParent, pos[2] or pos[1], pos[3] or 0, pos[4] or 0)
    elseif ChatFrame1 then
        f:SetPoint("BOTTOMLEFT", ChatFrame1, "TOPLEFT", 0, 34)
    else
        f:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 20, 260)
    end
    f:Hide()
end

-- What the bar should show right now: nil (hidden), "queued" or "live".
function Bar:Mode()
    local unsent, sent = ns.UI.QueueSummary()
    local H = SevenTVEmotes_Helper
    local compressChanged = ns.loadedCompress ~= nil and (ns.db.compress and true or false) ~= ns.loadedCompress
    local busy = type(H) == "table" and (type(H.progress) == "table"
        or (not compressChanged and type(H.storage) == "table" and H.storage.format
            and H.storage.format ~= (ns.db.compress and "blp" or "tga")))
    if self.manual or sent > 0 or busy then return "live", unsent end
    if unsent > 0 or compressChanged then return "queued", unsent end
    return nil, unsent
end

function Bar:Refresh()
    if not ns.db then return end
    local mode, unsent = self:Mode()
    if not mode or (self.dismissed and not self.manual) then
        if self.frame then self.frame:Hide() end
        return
    end
    self:Create()
    local f = self.frame
    local state = ns.HelperState()
    if mode == "queued" then
        f.live:Hide()
        f.fallback:SetText("")
        local what = unsent > 0 and format("%d emote request%s queued", unsent, unsent == 1 and "" or "s")
            or "Compression setting changed"
        ns.lastQueuedText = what
        f.title:SetText(what)
        f.sub:SetText("Send when you're ready, or log out to send.")
        f.button.label:SetText("Send")
        f.button.tip = "Reloads your UI so the helper app receives your requests. They're also sent automatically when you log out."
    else
        f.title:SetText("")
        f.sub:SetText("")
        if Board() then
            f.live:Show()
            f.fallback:SetText(state == "ok" and "Waiting for the helper app...\n(if this stays, restart WoW once)"
                or "The helper app isn't running.")
        else
            f.live:Hide()
            f.fallback:SetText(state == "missing" and "The 7TV Emotes Helper app isn't installed."
                or "The helper is working. Reload in a minute to check.")
        end
        f.button.label:SetText(unsent > 0 and "Send" or "Reload")
        f.button.tip = unsent > 0 and "Reloads your UI so the helper app receives your new requests."
            or "Reload once the helper says your emotes are ready."
    end
    f:Show()
end

function Bar:ShowManual()
    self.manual, self.dismissed = true, false
    self:Refresh()
end

-- Re-show when something new is queued.
table.insert(ns.queueListeners, function(reason)
    if reason == "added" then Bar.dismissed = false end
    Bar:Refresh()
end)

---------------------------------------------------------------------------
-- /7tv texturetest: shows the helper's test images in each texture format, to find out
-- which formats this WoW client loads from addon folders.
---------------------------------------------------------------------------
local TESTS = {
    { "t_tga.tga", "TGA (uncompressed)" },
    { "t_dxt5m.blp", "BLP DXT5 + mipmaps (what compression uses)" },
    { "t_dxt5.blp", "BLP DXT5, no mipmaps" },
    { "t_argb.blp", "BLP uncompressed" },
    { "t_pal.blp", "BLP palette" },
}

function ns.ShowTextureTest()
    local f = ns.testFrame
    if not f then
        f = CreateFrame("Frame", "SevenTVEmotesTextureTest", UIParent, "BackdropTemplate")
        ns.testFrame = f
        f:SetSize(300, 64 + #TESTS * 46)
        f:SetPoint("CENTER")
        f:SetFrameStrata("DIALOG")
        f:EnableMouse(true)
        f:SetMovable(true)
        f:RegisterForDrag("LeftButton")
        f:SetScript("OnDragStart", f.StartMoving)
        f:SetScript("OnDragStop", f.StopMovingOrSizing)
        f:SetBackdrop({ bgFile = "Interface\\Buttons\\WHITE8X8", edgeFile = "Interface\\Buttons\\WHITE8X8", edgeSize = 1 })
        f:SetBackdropColor(0.05, 0.05, 0.07, 0.95)
        f:SetBackdropBorderColor(ACCENT[1], ACCENT[2], ACCENT[3], 0.6)
        local title = f:CreateFontString(nil, "OVERLAY", "GameFontNormal")
        title:SetPoint("TOPLEFT", 12, -10)
        title:SetText("|cff29b6f67TV|r texture test")
        local close = SmallButton(f, "x", 16, 16, function() f:Hide() end)
        close:SetPoint("TOPRIGHT", -4, -4)
        f.rows = {}
        for i, t in ipairs(TESTS) do
            local y = -34 - (i - 1) * 46
            local label = f:CreateFontString(nil, "OVERLAY", "GameFontHighlightSmall")
            label:SetPoint("TOPLEFT", 12, y)
            label:SetText(i .. ". " .. t[2])
            local empty = f:CreateFontString(nil, "BACKGROUND", "GameFontDisableSmall")
            empty:SetPoint("TOPLEFT", 26, y - 22)
            empty:SetText("(nothing shown = this format doesn't load)")
            local tex = f:CreateTexture(nil, "ARTWORK")
            tex:SetSize(256, 32)
            tex:SetPoint("TOPLEFT", 22, y - 14)
            f.rows[i] = tex
        end
        local note = f:CreateFontString(nil, "OVERLAY", "GameFontDisableSmall")
        note:SetPoint("BOTTOMLEFT", 12, 10)
        note:SetWidth(276)
        note:SetJustifyH("LEFT")
        note:SetText("Each row should show a dark bar with its name.")
    end
    local results = {}
    for i, t in ipairs(TESTS) do
        local ok = f.rows[i]:SetTexture("Interface\\AddOns\\SevenTVEmotes_Local\\test\\" .. t[1])
        results[#results + 1] = format("%d=%s", i, tostring(ok))
    end
    f:Show()
    ns.Print("Texture test shown. If a row is empty, that format doesn't load (restart WoW once first if the helper was just updated). SetTexture results: " .. table.concat(results, " "))
end
