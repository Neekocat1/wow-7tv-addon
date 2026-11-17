-- SevenTVEmotes - the /7tv window
-- Tabs: Emotes (browse/manage), Add (emotes and channels), Downloads (helper progress), Settings.
-- Built only from plain frames, textures and font strings (no Blizzard templates beyond
-- BackdropTemplate) so it doesn't break when templates change between client versions.
local ADDON_NAME, ns = ...

local floor, format, max, min = math.floor, string.format, math.max, math.min
local unpack = unpack or table.unpack

local UI = {}
ns.UI = UI

local W, H = 720, 520
local ACCENT = { 0.16, 0.71, 0.96 }
local BG = { 0.06, 0.06, 0.08, 0.97 }
local PANEL = { 0.09, 0.09, 0.11, 1 }
local LINE = { 0.25, 0.25, 0.3, 1 }
local GREEN, RED, YELLOW, GREY, ORANGE = "|cff40ff40", "|cffff6060", "|cffffd100", "|cff9a9a9a", "|cffffa040"
local WHITE8 = "Interface\\Buttons\\WHITE8X8"

---------------------------------------------------------------------------
-- Small helpers
---------------------------------------------------------------------------
local function Backdrop(f, bg, border)
    f:SetBackdrop({ bgFile = WHITE8, edgeFile = WHITE8, edgeSize = 1 })
    f:SetBackdropColor(unpack(bg))
    f:SetBackdropBorderColor(unpack(border or LINE))
end

local function Text(parent, font, text, justify)
    local fs = parent:CreateFontString(nil, "OVERLAY", font or "GameFontHighlight")
    if text then fs:SetText(text) end
    fs:SetJustifyH(justify or "LEFT")
    return fs
end

local function Bytes(n)
    n = tonumber(n) or 0
    if n >= 1024 * 1024 * 1024 then return format("%.1f GB", n / 1024 ^ 3) end
    if n >= 1024 * 1024 then return format("%.1f MB", n / 1024 ^ 2) end
    if n >= 1024 then return format("%d KB", floor(n / 1024 + 0.5)) end
    return format("%d B", n)
end
UI.Bytes = Bytes

local function Ago(t)
    local s = time() - (tonumber(t) or 0)
    if s < 60 then return "just now" end
    if s < 3600 then return format("%d min ago", floor(s / 60)) end
    if s < 86400 then return format("%d h ago", floor(s / 3600)) end
    return format("%d days ago", floor(s / 86400))
end
UI.Ago = Ago

local function Button(parent, label, w, h, onClick, primary)
    local b = CreateFrame("Button", nil, parent, "BackdropTemplate")
    b:SetSize(w, h or 22)
    local bg = primary and { ACCENT[1] * 0.45, ACCENT[2] * 0.45, ACCENT[3] * 0.45, 1 } or { 0.13, 0.13, 0.16, 1 }
    Backdrop(b, bg)
    b.label = Text(b, "GameFontHighlightSmall", label, "CENTER")
    b.label:SetPoint("CENTER")
    b:SetScript("OnEnter", function(s)
        if s:IsEnabled() then s:SetBackdropBorderColor(ACCENT[1], ACCENT[2], ACCENT[3], 1) end
        if s.tip then
            GameTooltip:SetOwner(s, "ANCHOR_TOP")
            GameTooltip:AddLine(s.tip, 1, 1, 1, true)
            GameTooltip:Show()
        end
    end)
    b:SetScript("OnLeave", function(s)
        s:SetBackdropBorderColor(unpack(LINE))
        if s.tip then GameTooltip:Hide() end
    end)
    b:SetScript("OnClick", onClick)
    function b:SetLabel(t) self.label:SetText(t) end
    function b:SetActive(on)
        if on then self:Enable(); self.label:SetFontObject("GameFontHighlightSmall")
        else self:Disable(); self.label:SetFontObject("GameFontDisableSmall") end
    end
    return b
end

local function Input(parent, w, placeholder)
    local e = CreateFrame("EditBox", nil, parent, "BackdropTemplate")
    e:SetSize(w, 24)
    e:SetAutoFocus(false)
    e:SetFontObject(ChatFontNormal)
    e:SetTextInsets(7, 7, 0, 0)
    e:SetMaxLetters(255)
    Backdrop(e, { 0.03, 0.03, 0.04, 1 })
    e.ph = Text(e, "GameFontDisableSmall", placeholder)
    e.ph:SetPoint("LEFT", 8, 0)
    e:SetScript("OnTextChanged", function(s) s.ph:SetShown(s:GetText() == "") ; if s.onChange then s.onChange(s) end end)
    e:SetScript("OnEscapePressed", function(s) s:ClearFocus() end)
    e:SetScript("OnEditFocusGained", function(s) s:SetBackdropBorderColor(ACCENT[1], ACCENT[2], ACCENT[3], 1) end)
    e:SetScript("OnEditFocusLost", function(s) s:SetBackdropBorderColor(unpack(LINE)) end)
    return e
end

local function Toggle(parent, label, get, set, tip)
    local b = CreateFrame("Button", nil, parent, "BackdropTemplate")
    b:SetSize(16, 16)
    Backdrop(b, { 0.03, 0.03, 0.04, 1 })
    b.check = b:CreateTexture(nil, "ARTWORK")
    b.check:SetPoint("TOPLEFT", 3, -3)
    b.check:SetPoint("BOTTOMRIGHT", -3, 3)
    b.check:SetColorTexture(ACCENT[1], ACCENT[2], ACCENT[3], 1)
    b.text = Text(parent, "GameFontHighlight", label)
    b.text:SetPoint("LEFT", b, "RIGHT", 8, 0)
    if tip then
        b.note = Text(parent, "GameFontDisableSmall", tip)
        b.note:SetPoint("TOPLEFT", b.text, "BOTTOMLEFT", 0, -2)
        b.note:SetWidth(560)
    end
    function b:Refresh() self.check:SetShown(get() and true or false) end
    b:SetScript("OnClick", function(self) set(not get()); self:Refresh() end)
    b:Refresh()
    return b
end

-- Thin scrollbar; onChange(value) receives an integer offset in [0, maxValue].
local function Scrollbar(parent, onChange)
    local bar = CreateFrame("Frame", nil, parent)
    bar:SetWidth(8)
    bar:EnableMouse(true)
    local track = bar:CreateTexture(nil, "BACKGROUND")
    track:SetAllPoints()
    track:SetColorTexture(1, 1, 1, 0.05)
    local thumb = CreateFrame("Button", nil, bar)
    thumb:SetWidth(8)
    local tt = thumb:CreateTexture(nil, "ARTWORK")
    tt:SetAllPoints()
    tt:SetColorTexture(ACCENT[1], ACCENT[2], ACCENT[3], 0.6)
    bar.value, bar.maxValue, bar.frac = 0, 0, 1

    function bar:Layout()
        local h = self:GetHeight() or 0
        if self.maxValue <= 0 or h <= 0 then thumb:Hide(); return end
        thumb:Show()
        local th = max(24, h * self.frac)
        thumb:SetHeight(th)
        thumb:ClearAllPoints()
        thumb:SetPoint("TOP", self, "TOP", 0, -((h - th) * self.value / self.maxValue))
    end
    function bar:SetRange(maxValue, frac)
        self.maxValue, self.frac = max(0, maxValue), min(1, frac or 1)
        if self.value > self.maxValue then self.value = self.maxValue end
        self:Layout()
    end
    function bar:SetValue(v, silent)
        v = min(max(0, floor(v + 0.5)), self.maxValue)
        if v ~= self.value then
            self.value = v
            self:Layout()
            if not silent then onChange(v) end
        end
    end
    local function cursorY()
        local _, y = GetCursorPosition()
        return y / UIParent:GetEffectiveScale()
    end
    thumb:SetScript("OnMouseDown", function() thumb.drag, thumb.y0, thumb.v0 = true, cursorY(), bar.value end)
    thumb:SetScript("OnMouseUp", function() thumb.drag = false end)
    thumb:SetScript("OnUpdate", function()
        if not thumb.drag then return end
        local room = bar:GetHeight() - thumb:GetHeight()
        if room > 0 then bar:SetValue(thumb.v0 + (thumb.y0 - cursorY()) / room * bar.maxValue) end
    end)
    bar:SetScript("OnMouseDown", function()
        local top = bar:GetTop()
        local h = bar:GetHeight()
        if top and h > 0 then bar:SetValue((top - cursorY()) / h * bar.maxValue) end
    end)
    bar:SetScript("OnSizeChanged", function() bar:Layout() end)
    return bar
end

-- Fit an emote into a w x h box, keeping its aspect ratio.
local function FitSize(e, boxW, boxH)
    local w, h = boxH * e.w / e.h, boxH
    if w > boxW then w, h = boxW, boxW * e.h / e.w end
    return max(1, floor(w + 0.5)), max(1, floor(h + 0.5))
end

local function SetEmoteTexture(tex, e)
    tex:SetTexture(e.path)
    local l, r, t, b = ns.Coords(e, e.frames and 0 or nil)
    tex:SetTexCoord(l / e.tw, r / e.tw, t / e.th, b / e.th)
    ns.animTextures[tex] = e.frames and e or nil
end

local function RequestLabel(r)
    local ref = r.ref or ""
    if ref:find("7tv", 1, true) or #ref >= 24 then ref = "emote from 7TV link" end
    if r.kind == "add" then return "Add " .. (r.alias and (r.alias .. " (" .. ref .. ")") or ref) end
    if r.kind == "remove" then return "Remove " .. ref end
    if r.kind == "channel" then return "Add " .. ref .. "'s channel" end
    if r.kind == "unchannel" then return "Remove " .. ref .. "'s channel" end
    return tostring(r.kind) .. " " .. ref
end

local function PendingRemoval(name)
    for _, r in ipairs(ns.db.queue) do
        if r.kind == "remove" and r.ref == name then return true end
    end
end

---------------------------------------------------------------------------
-- Request status (what the Downloads tab and footer show)
---------------------------------------------------------------------------
-- Returns label, colour, detail for a queued request.
function UI.RequestStatus(r)
    local state, H = ns.HelperState()
    if ns.sessionIds[r.id] then
        return "Not sent yet", YELLOW, "Reload to send it to the helper."
    end
    if state == "missing" then
        return "Needs helper", RED, "Install the 7TV Emotes Helper app to add emotes."
    end
    if H and type(H.retrying) == "table" and H.retrying[r.id] then
        return "Retrying", ORANGE, H.retrying[r.id]
    end
    if state == "stale" then
        return "Waiting for helper", GREY, "The helper app isn't running. Start it, then reload."
    end
    return "In progress", "|cff29b6f6", format("Sent %s. Reload to see the result.", Ago(r.t))
end

function UI.QueueSummary()
    local unsent, sent = 0, 0
    for _, r in ipairs(ns.db.queue) do
        if ns.sessionIds[r.id] then unsent = unsent + 1 else sent = sent + 1 end
    end
    return unsent, sent
end

---------------------------------------------------------------------------
-- Emotes tab
---------------------------------------------------------------------------
local CELL_W, CELL_H, ICON_H = 84, 78, 44

local function BuildEmotesPage(page)
    local p = {}
    p.search = Input(page, 220, "Search emotes")
    p.search:SetPoint("TOPLEFT", 0, 0)
    p.search.onChange = function() p.offset = 0; p:Refresh() end

    p.filter = "all"
    local filters = { { "all", "All" }, { "mine", "Mine" }, { "animated", "Animated" }, { "hidden", "Hidden" } }
    p.filterButtons = {}
    local prev
    for _, f in ipairs(filters) do
        local b = Button(page, f[2], 64, 24, function() p.filter = f[1]; p.offset = 0; p:Refresh() end)
        b:SetPoint("LEFT", prev or p.search, "RIGHT", prev and 4 or 8, 0)
        b.key = f[1]
        p.filterButtons[#p.filterButtons + 1] = b
        prev = b
    end
    p.count = Text(page, "GameFontDisableSmall", "", "RIGHT")
    p.count:SetPoint("RIGHT", page, "TOPRIGHT", 0, -12)

    -- grid
    local grid = CreateFrame("Frame", nil, page, "BackdropTemplate")
    Backdrop(grid, PANEL)
    grid:SetPoint("TOPLEFT", 0, -32)
    grid:SetPoint("BOTTOMLEFT", 0, 0)
    grid:SetWidth(CELL_W * 5 + 22)
    grid:EnableMouseWheel(true)
    p.cols, p.rows = 5, floor((H - 72 - 50 - 32 - 8) / CELL_H)
    p.cells = {}
    for i = 1, p.cols * p.rows do
        local c = CreateFrame("Button", nil, grid, "BackdropTemplate")
        c:SetSize(CELL_W - 4, CELL_H - 4)
        local col, row = (i - 1) % p.cols, floor((i - 1) / p.cols)
        c:SetPoint("TOPLEFT", 6 + col * CELL_W, -6 - row * CELL_H)
        Backdrop(c, { 0, 0, 0, 0 }, { 0, 0, 0, 0 })
        c.icon = c:CreateTexture(nil, "ARTWORK")
        c.icon:SetPoint("CENTER", 0, 8)
        c.name = Text(c, "GameFontHighlightSmall", "", "CENTER")
        c.name:SetPoint("BOTTOMLEFT", 2, 4)
        c.name:SetPoint("BOTTOMRIGHT", -2, 4)
        c.name:SetWordWrap(false)
        local hl = c:CreateTexture(nil, "HIGHLIGHT")
        hl:SetAllPoints()
        hl:SetColorTexture(1, 1, 1, 0.06)
        c:SetScript("OnClick", function(self) p.selected = self.emote and self.emote.name; p:Refresh() end)
        p.cells[i] = c
    end
    p.bar = Scrollbar(grid, function(v) p.offset = v; p:RefreshGrid() end)
    p.bar:SetPoint("TOPRIGHT", -4, -6)
    p.bar:SetPoint("BOTTOMRIGHT", -4, 6)
    grid:SetScript("OnMouseWheel", function(_, delta) p.bar:SetValue(p.bar.value - delta) end)
    p.empty = Text(grid, "GameFontDisable", "", "CENTER")
    p.empty:SetPoint("CENTER")
    p.empty:SetWidth(360)

    -- detail pane
    local d = CreateFrame("Frame", nil, page, "BackdropTemplate")
    Backdrop(d, PANEL)
    d:SetPoint("TOPLEFT", grid, "TOPRIGHT", 8, 0)
    d:SetPoint("BOTTOMRIGHT", 0, 0)
    p.detail = d
    d.preview = d:CreateTexture(nil, "ARTWORK")
    d.preview:SetPoint("TOP", 0, -16)
    d.name = Text(d, "GameFontNormalLarge", "", "CENTER")
    d.name:SetPoint("TOP", 0, -128)
    d.name:SetWidth(200)
    d.info = Text(d, "GameFontHighlightSmall", "", "CENTER")
    d.info:SetPoint("TOP", d.name, "BOTTOM", 0, -6)
    d.info:SetWidth(200)
    d.copy = Input(d, 180, "")
    d.copy:SetPoint("TOP", d.info, "BOTTOM", 0, -12)
    d.copy:SetScript("OnEditFocusGained", function(s) s:HighlightText() end)
    d.copy:SetScript("OnTextChanged", function(s, user) if user and p.selected then s:SetText(p.selected); s:HighlightText() end end)
    d.copyHint = Text(d, "GameFontDisableSmall", "Click, then Ctrl+C to copy the name", "CENTER")
    d.copyHint:SetPoint("TOP", d.copy, "BOTTOM", 0, -3)
    d.hide = Button(d, "Hide in chat", 180, 24, function()
        local n = p.selected
        if not n then return end
        ns.db.disabled[n] = (not ns.db.disabled[n]) or nil
        p:Refresh()
    end)
    d.hide:SetPoint("TOP", d.copyHint, "BOTTOM", 0, -14)
    d.hide.tip = "Hidden emotes stay as plain text in chat and don't show in autocomplete."
    d.remove = Button(d, "Remove", 180, 24, function()
        local n = p.selected
        if not n then return end
        ns.QueueRequest("remove", n, nil, "Removing " .. n, true)
        p:Refresh()
    end)
    d.remove:SetPoint("TOP", d.hide, "BOTTOM", 0, -6)
    d.none = Text(d, "GameFontDisable", "Select an emote", "CENTER")
    d.none:SetPoint("CENTER")

    function p:List()
        local out, q = {}, (self.search:GetText() or ""):lower()
        for i, name in ipairs(ns.names) do
            local e = ns.emotes[name]
            if q == "" or ns.lnames[i]:find(q, 1, true) then
                local f = self.filter
                if f == "all" or (f == "mine" and e.pack == "My emotes") or (f == "animated" and e.frames)
                    or (f == "hidden" and ns.db.disabled[name]) then
                    out[#out + 1] = e
                end
            end
        end
        return out
    end

    function p:RefreshGrid()
        local list, cols = self.list, self.cols
        local start = (self.offset or 0) * cols
        for i, c in ipairs(self.cells) do
            local e = list[start + i]
            c.emote = e
            if e then
                local w, h = FitSize(e, CELL_W - 12, ICON_H)
                c.icon:SetSize(w, h)
                SetEmoteTexture(c.icon, e)
                local hidden = ns.db.disabled[e.name]
                c.icon:SetDesaturated(hidden and true or false)
                c.icon:SetAlpha(hidden and 0.4 or 1)
                c.name:SetText((hidden and GREY or "") .. e.name)
                if self.selected == e.name then
                    c:SetBackdropBorderColor(ACCENT[1], ACCENT[2], ACCENT[3], 1)
                    c:SetBackdropColor(ACCENT[1], ACCENT[2], ACCENT[3], 0.12)
                else
                    c:SetBackdropBorderColor(0, 0, 0, 0)
                    c:SetBackdropColor(0, 0, 0, 0)
                end
                c:Show()
            else
                ns.animTextures[c.icon] = nil
                c:Hide()
            end
        end
    end

    function p:RefreshDetail()
        local e = self.selected and ns.emotes[self.selected]
        local show = e ~= nil
        for _, r in ipairs({ d.preview, d.name, d.info, d.copy, d.copyHint, d.hide, d.remove }) do r:SetShown(show) end
        d.none:SetShown(not show)
        if not e then ns.animTextures[d.preview] = nil; return end
        local w, h = FitSize(e, 200, 96)
        d.preview:SetSize(w, h)
        SetEmoteTexture(d.preview, e)
        d.name:SetText(e.name)
        local H = SevenTVEmotes_Helper
        local origin = (e.pack == "My emotes" and type(H) == "table" and type(H.origin) == "table" and H.origin[e.name])
            or (e.pack == "Bundled" and "Comes with the addon") or e.pack
        d.info:SetText(format("%s\n%s", origin or "", e.frames and format("Animated, %d frames", e.frames) or "Static"))
        d.copy:SetText(e.name)
        d.hide:SetLabel(ns.db.disabled[e.name] and "Show in chat" or "Hide in chat")
        local canRemove = e.pack == "My emotes"
        if not canRemove then
            d.remove:SetLabel("Can't remove (built in)")
            d.remove:SetActive(false)
        elseif PendingRemoval(e.name) then
            d.remove:SetLabel("Removal queued")
            d.remove:SetActive(false)
        else
            d.remove:SetLabel("Remove")
            d.remove:SetActive(ns.HelperState() ~= "missing")
        end
    end

    function p:Refresh()
        self.list = self:List()
        for _, b in ipairs(self.filterButtons) do
            if b.key == self.filter then
                b:SetBackdropColor(ACCENT[1] * 0.45, ACCENT[2] * 0.45, ACCENT[3] * 0.45, 1)
            else
                b:SetBackdropColor(0.13, 0.13, 0.16, 1)
            end
        end
        local rowsTotal = math.ceil(#self.list / self.cols)
        self.bar:SetRange(rowsTotal - self.rows, self.rows / max(1, rowsTotal))
        self.offset = min(self.offset or 0, max(0, rowsTotal - self.rows))
        self.bar:SetValue(self.offset, true)
        self.count:SetText(format("%d of %d emotes", #self.list, ns.count))
        if #self.list == 0 then
            self.empty:SetText(ns.count == 0 and "No emotes yet.\nUse the Add tab to add some." or "Nothing matches.")
            self.empty:Show()
        else
            self.empty:Hide()
        end
        self:RefreshGrid()
        self:RefreshDetail()
    end

    function p:OnHide()
        for _, c in ipairs(self.cells) do ns.animTextures[c.icon] = nil end
        ns.animTextures[d.preview] = nil
    end
    return p
end

---------------------------------------------------------------------------
-- Add tab
---------------------------------------------------------------------------
local function BuildAddPage(page)
    local p = {}
    local y = 0
    local function heading(text)
        local fs = Text(page, "GameFontNormal", text)
        fs:SetPoint("TOPLEFT", 0, y)
        y = y - 22
        return fs
    end

    p.banner = Text(page, "GameFontHighlight", "")
    p.banner:SetPoint("TOPLEFT", 0, 0)
    p.banner:SetWidth(690)
    y = -34

    heading("Add an emote")
    p.link = Input(page, 440, "Paste a 7tv.app emote link, or type the emote's exact name")
    p.link:SetPoint("TOPLEFT", 0, y)
    p.alias = Input(page, 130, "Rename (optional)")
    p.alias:SetPoint("LEFT", p.link, "RIGHT", 6, 0)
    p.addBtn = Button(page, "Add emote", 110, 24, function() p:AddEmote() end, true)
    p.addBtn:SetPoint("LEFT", p.alias, "RIGHT", 6, 0)
    p.link:SetScript("OnEnterPressed", function() p:AddEmote() end)
    p.alias:SetScript("OnEnterPressed", function() p:AddEmote() end)
    y = y - 30
    local hint = Text(page, "GameFontDisableSmall",
        "Find an emote on 7tv.app, copy the address from your browser and paste it here with Ctrl+V. " ..
        "Links to a whole emote set work too.")
    hint:SetPoint("TOPLEFT", 0, y)
    hint:SetWidth(690)
    y = y - 34

    heading("Add a streamer's emotes")
    p.channel = Input(page, 440, "Twitch channel name")
    p.channel:SetPoint("TOPLEFT", 0, y)
    p.chanBtn = Button(page, "Add channel", 110, 24, function() p:AddChannel() end, true)
    p.chanBtn:SetPoint("LEFT", p.channel, "RIGHT", 6, 0)
    p.channel:SetScript("OnEnterPressed", function() p:AddChannel() end)
    y = y - 30
    p.feedback = Text(page, "GameFontHighlight", "")
    p.feedback:SetPoint("TOPLEFT", 0, y)
    p.feedback:SetWidth(690)
    y = y - 30

    heading("Your channels")
    p.chanRows = {}
    for i = 1, 5 do
        local row = CreateFrame("Frame", nil, page, "BackdropTemplate")
        row:SetSize(560, 26)
        row:SetPoint("TOPLEFT", 0, y - (i - 1) * 28)
        Backdrop(row, PANEL)
        row.text = Text(row, "GameFontHighlight", "")
        row.text:SetPoint("LEFT", 8, 0)
        row.status = Text(row, "GameFontDisableSmall", "", "RIGHT")
        row.status:SetPoint("RIGHT", -100, 0)
        row.btn = Button(row, "Remove", 84, 20, function(self)
            ns.QueueRequest("unchannel", self.login, nil, "Removing " .. self.login .. "'s channel", true)
            UI:Refresh()
        end)
        row.btn:SetPoint("RIGHT", -4, 0)
        p.chanRows[i] = row
    end
    p.more = Text(page, "GameFontDisableSmall", "")
    p.more:SetPoint("TOPLEFT", 0, y - 5 * 28)

    local function feedback(ok, msg)
        p.feedback:SetText((ok and GREEN or RED) .. msg .. "|r")
    end

    function p:AddEmote()
        local ref = (self.link:GetText() or ""):gsub("^%s+", ""):gsub("%s+$", "")
        local alias = (self.alias:GetText() or ""):gsub("%s", "")
        if ref == "" then return feedback(false, "Paste a 7TV link or type an emote name first.") end
        if alias:find("|", 1, true) then return feedback(false, "Names can't contain |.") end
        if alias == "" then alias = nil end
        local label = alias or ((ref:find("7tv", 1, true) or #ref >= 24) and "Emote" or ref)
        local ok, msg = ns.QueueRequest("add", ref, alias, label, true)
        feedback(ok, msg)
        if ok then self.link:SetText(""); self.alias:SetText(""); self.link:ClearFocus(); self.alias:ClearFocus() end
        UI:Refresh()
    end

    function p:AddChannel()
        local login = (self.channel:GetText() or ""):gsub("^%s*@?", ""):gsub("%s+$", ""):lower()
        if login == "" or login:find("%s") then return feedback(false, "Type a Twitch channel name.") end
        local ok, msg = ns.QueueRequest("channel", login, nil, login .. "'s channel", true)
        feedback(ok, msg)
        if ok then self.channel:SetText(""); self.channel:ClearFocus() end
        UI:Refresh()
    end

    function p:Refresh()
        local state, H = ns.HelperState()
        local enabled = state ~= "missing"
        if state == "missing" then
            self.banner:SetText(RED .. "The 7TV Emotes Helper app isn't installed.|r Adding emotes needs it: install it once and it runs in the background.")
        elseif state == "stale" then
            self.banner:SetText(YELLOW .. "The helper app hasn't checked in recently.|r You can still queue emotes; start the helper from your system tray, then reload.")
        else
            self.banner:SetText(GREY .. "Emotes you add here are fetched by the helper app the next time you reload.|r")
        end
        self.addBtn:SetActive(enabled)
        self.chanBtn:SetActive(enabled)

        -- channels: helper's list + queued changes
        local rows, seen = {}, {}
        local pendingUn, pendingAdd = {}, {}
        for _, r in ipairs(ns.db.queue) do
            if r.kind == "unchannel" then pendingUn[r.ref] = true end
            if r.kind == "channel" then pendingAdd[r.ref] = true end
        end
        if H and type(H.channels) == "table" then
            for _, login in ipairs(H.channels) do
                seen[login] = true
                rows[#rows + 1] = { login = login, status = pendingUn[login] and (YELLOW .. "removal queued") or "" }
            end
        end
        for login in pairs(pendingAdd) do
            if not seen[login] then rows[#rows + 1] = { login = login, status = YELLOW .. "queued", queued = true } end
        end
        table.sort(rows, function(a, b) return a.login < b.login end)
        for i, row in ipairs(self.chanRows) do
            local r = rows[i]
            if r then
                row.text:SetText(r.login)
                row.status:SetText(r.status)
                row.btn.login = r.login
                row.btn:SetShown(not r.queued)
                row.btn:SetActive(enabled and not pendingUn[r.login])
                row:Show()
            else
                row:Hide()
            end
        end
        if #rows == 0 then
            self.more:SetText("None yet.")
            self.more:ClearAllPoints()
            self.more:SetPoint("TOPLEFT", self.chanRows[1], "TOPLEFT", 0, 0)
        else
            self.more:SetText(#rows > #self.chanRows and format("... and %d more", #rows - #self.chanRows) or "")
            self.more:ClearAllPoints()
            self.more:SetPoint("TOPLEFT", self.chanRows[#self.chanRows], "BOTTOMLEFT", 0, -6)
        end
    end
    return p
end

---------------------------------------------------------------------------
-- Downloads tab
---------------------------------------------------------------------------
local ROW_H = 40

local function BuildDownloadsPage(page)
    local p = {}
    local status = CreateFrame("Frame", nil, page, "BackdropTemplate")
    Backdrop(status, PANEL)
    status:SetPoint("TOPLEFT", 0, 0)
    status:SetPoint("TOPRIGHT", 0, 0)
    status:SetHeight(58)
    p.dot = status:CreateTexture(nil, "ARTWORK")
    p.dot:SetSize(10, 10)
    p.dot:SetPoint("TOPLEFT", 12, -14)
    p.title = Text(status, "GameFontNormal", "")
    p.title:SetPoint("LEFT", p.dot, "RIGHT", 8, 0)
    p.sub = Text(status, "GameFontHighlightSmall", "")
    p.sub:SetPoint("TOPLEFT", 30, -32)
    p.sub:SetWidth(430)
    p.live = ns.CreateLiveImage(status, 192, 24)
    p.live:SetPoint("RIGHT", -10, 0)

    local list = CreateFrame("Frame", nil, page, "BackdropTemplate")
    Backdrop(list, PANEL)
    list:SetPoint("TOPLEFT", status, "BOTTOMLEFT", 0, -8)
    list:SetPoint("BOTTOMRIGHT", 0, 28)
    list:EnableMouseWheel(true)
    p.visible = floor((H - 72 - 50 - 58 - 8 - 28 - 8) / ROW_H)
    p.rows = {}
    for i = 1, p.visible do
        local row = CreateFrame("Frame", nil, list)
        row:SetHeight(ROW_H)
        row:SetPoint("TOPLEFT", 6, -4 - (i - 1) * ROW_H)
        row:SetPoint("RIGHT", list, "RIGHT", -18, 0)
        row.state = Text(row, "GameFontNormalSmall", "")
        row.state:SetPoint("TOPLEFT", 4, -5)
        row.state:SetWidth(120)
        row.title = Text(row, "GameFontHighlight", "")
        row.title:SetPoint("TOPLEFT", 130, -4)
        row.title:SetPoint("RIGHT", row, "RIGHT", -96, 0)
        row.title:SetWordWrap(false)
        row.detail = Text(row, "GameFontDisableSmall", "")
        row.detail:SetPoint("TOPLEFT", 130, -21)
        row.detail:SetPoint("RIGHT", row, "RIGHT", -96, 0)
        row.detail:SetWordWrap(false)
        row.btn = Button(row, "Cancel", 80, 20, function(self) ns.CancelRequest(self.reqId); UI:Refresh() end)
        row.btn:SetPoint("RIGHT", 0, 0)
        local sep = row:CreateTexture(nil, "BACKGROUND")
        sep:SetPoint("BOTTOMLEFT")
        sep:SetPoint("BOTTOMRIGHT")
        sep:SetHeight(1)
        sep:SetColorTexture(1, 1, 1, 0.05)
        p.rows[i] = row
    end
    p.bar = Scrollbar(list, function(v) p.offset = v; p:RefreshRows() end)
    p.bar:SetPoint("TOPRIGHT", -4, -6)
    p.bar:SetPoint("BOTTOMRIGHT", -4, 6)
    list:SetScript("OnMouseWheel", function(_, delta) p.bar:SetValue(p.bar.value - delta) end)
    p.empty = Text(list, "GameFontDisable", "No downloads yet. Add emotes from the Add tab.", "CENTER")
    p.empty:SetPoint("CENTER")

    p.note = Text(page, "GameFontDisableSmall",
        "This list updates when you reload. The live status (top right, and the 7TV bar by your chat) updates every few seconds.")
    p.note:SetPoint("BOTTOMLEFT", 0, 6)
    p.clear = Button(page, "Clear history", 100, 20, function()
        local h = ns.db.history
        for i = #h, 1, -1 do h[i] = nil end
        UI:Refresh()
    end)
    p.clear:SetPoint("BOTTOMRIGHT", 0, 2)

    function p:Items()
        local items = {}
        for _, r in ipairs(ns.db.queue) do
            local label, color, detail = UI.RequestStatus(r)
            items[#items + 1] = { state = color .. label .. "|r", title = RequestLabel(r), detail = detail, reqId = r.id }
        end
        local H = SevenTVEmotes_Helper
        if ns.invisible and #ns.invisible > 0 then
            items[#items + 1] = { state = ORANGE .. "Needs restart|r",
                title = format("%d emote%s can't be shown until you restart WoW", #ns.invisible, #ns.invisible == 1 and "" or "s"),
                detail = "WoW only sees image files that existed when it started." }
        end
        if type(H) == "table" then
            local pr = H.progress
            if type(pr) == "table" and (pr.total or 0) > 0 then
                items[#items + 1] = { state = "|cff29b6f6Downloading|r",
                    title = format("%d of %d emotes downloaded", pr.done or 0, pr.total),
                    detail = format("As of %s. Reload to see more.", Ago(pr.t)) }
            end
            if type(H.notice) == "string" and H.notice ~= "" then
                items[#items + 1] = { state = RED .. "Problem|r", title = H.notice, detail = "" }
            end
            if type(H.waiting) == "table" and #H.waiting > 0 then
                local names = table.concat(H.waiting, ", ", 1, min(#H.waiting, 8))
                items[#items + 1] = { state = ORANGE .. "Needs restart|r",
                    title = format("%d emote%s ready after you restart WoW", #H.waiting, #H.waiting == 1 and "" or "s"),
                    detail = names .. (#H.waiting > 8 and ", ..." or "") }
            end
            local st = H.storage
            if type(st) == "table" and (st.converting or 0) > 0 then
                items[#items + 1] = { state = ORANGE .. "Converting|r",
                    title = format("%d emotes still use the old texture format", st.converting),
                    detail = "They switch over after you restart WoW." }
            end
        end
        for _, h in ipairs(ns.db.history) do
            items[#items + 1] = { state = h.ok and (GREEN .. "Done|r") or (RED .. "Failed|r"),
                title = h.msg, detail = Ago(h.t), name = h.ok and h.name }
        end
        return items
    end

    function p:RefreshRows()
        local start = self.offset or 0
        for i, row in ipairs(self.rows) do
            local it = self.items[start + i]
            if it then
                row.state:SetText(it.state)
                local e = it.name and ns.emotes[it.name]
                row.title:SetText((e and (ns.Tag(e, 18, e.frames and 0 or nil) .. " ") or "") .. it.title)
                row.detail:SetText(it.detail or "")
                row.btn.reqId = it.reqId
                row.btn:SetShown(it.reqId ~= nil)
                row:Show()
            else
                row:Hide()
            end
        end
    end

    function p:Refresh()
        local state, H = ns.HelperState()
        if state == "ok" then
            self.dot:SetColorTexture(0.25, 1, 0.25, 1)
            self.title:SetText("Helper app running")
            self.sub:SetText(format("Version %s, last check-in %s. Room for %d more emotes before the next WoW restart.",
                tostring(H.version), Ago(H.heartbeat), H.slotsFree or 0))
        elseif state == "stale" then
            self.dot:SetColorTexture(1, 0.82, 0, 1)
            self.title:SetText("Helper app not running")
            self.sub:SetText(format("Last seen %s. Start it from the Start menu or system tray; queued requests wait until it's back.", Ago(H.heartbeat)))
        else
            self.dot:SetColorTexture(1, 0.3, 0.3, 1)
            self.title:SetText("Helper app not installed")
            self.sub:SetText("The 7TV Emotes Helper downloads emotes for you. You can still use emotes that are already installed.")
        end
        if ns.Board and ns.Board() and state ~= "missing" then
            self.live:Show()
        else
            self.live:Hide()
        end
        self.items = self:Items()
        self.bar:SetRange(#self.items - self.visible, self.visible / max(1, #self.items))
        self.offset = min(self.offset or 0, max(0, #self.items - self.visible))
        self.bar:SetValue(self.offset, true)
        self.empty:SetShown(#self.items == 0)
        self.clear:SetShown(#ns.db.history > 0)
        self:RefreshRows()
    end
    return p
end

---------------------------------------------------------------------------
-- Settings tab
---------------------------------------------------------------------------
local function BuildSettingsPage(page)
    local p = {}
    local db = function() return ns.db end
    local y = 0
    local function heading(text)
        local fs = Text(page, "GameFontNormal", text)
        fs:SetPoint("TOPLEFT", 0, y)
        y = y - 24
    end
    local function toggle(key, label, tip, after)
        local t = Toggle(page, label, function() return db()[key] end, function(v)
            db()[key] = v
            if after then after(v) end
            UI:Refresh()
        end, tip)
        t:SetPoint("TOPLEFT", 0, y)
        y = y - (tip and 40 or 26)
        return t
    end

    heading("Chat")
    p.toggles = {
        toggle("enabled", "Show emotes in chat"),
        toggle("animate", "Animate emotes"),
        toggle("autocomplete", "Autocomplete", "Type : and part of a name in the chat box. Tab cycles, another : accepts."),
        toggle("links", "Hover tooltips", "Hover an emote in chat to see its name."),
        toggle("minimap", "Minimap button", nil, function(v) if ns.Minimap then ns.Minimap:SetShown(v) end end),
    }
    p.sizeLabel = Text(page, "GameFontHighlight", "")
    p.sizeLabel:SetPoint("TOPLEFT", 0, y - 4)
    p.minus = Button(page, "-", 24, 22, function() db().size = max(12, db().size - 2); UI:Refresh() end)
    p.minus:SetPoint("TOPLEFT", 160, y)
    p.plus = Button(page, "+", 24, 22, function() db().size = min(64, db().size + 2); UI:Refresh() end)
    p.plus:SetPoint("LEFT", p.minus, "RIGHT", 4, 0)
    p.sample = Text(page, "GameFontHighlight", "")
    p.sample:SetPoint("LEFT", p.plus, "RIGHT", 12, 0)
    y = y - 40

    heading("Storage")
    p.storage = Text(page, "GameFontHighlight", "")
    p.storage:SetPoint("TOPLEFT", 0, y)
    p.storage:SetWidth(690)
    y = y - 40
    p.compress = toggle("compress", "Compressed emotes",
        "Uses WoW's compressed texture format: about 4x smaller on disk and in memory, with slightly softer colors.")
    p.compressNote = Text(page, "GameFontHighlightSmall", "")
    p.compressNote:SetPoint("TOPLEFT", 24, y)
    p.compressNote:SetWidth(660)
    y = y - 34
    p.more = Button(page, "Open game settings page", 180, 22, function() ns.OpenSettings() end)
    p.more:SetPoint("TOPLEFT", 0, y)

    function p:Refresh()
        for _, t in ipairs(self.toggles) do t:Refresh() end
        self.compress:Refresh()
        self.sizeLabel:SetText(format("Emote size: %d", db().size))
        local e = ns.emotes[ns.names[1] or ""]
        self.sample:SetText(e and (ns.Tag(e, db().size, e.frames and 0 or nil) .. " " .. GREY .. "new messages|r") or "")

        local H = SevenTVEmotes_Helper
        local st = type(H) == "table" and type(H.storage) == "table" and H.storage or nil
        if not st then
            self.storage:SetText(GREY .. "Disk usage appears here once the helper app has run.|r")
            self.compressNote:SetText("")
            self.compress:SetEnabled(ns.HelperState() ~= "missing")
            return
        end
        local line = format("Your emotes use |cffffffff%s|r on disk (%d emotes, %s).", Bytes(st.bytes), st.emotes or 0,
            st.format == "blp" and "compressed" or "uncompressed")
        if st.cache then line = line .. format(" The helper's download cache uses another %s.", Bytes(st.cache)) end
        self.storage:SetText(line)

        local want = db().compress and "blp" or "tga"
        local note
        if st.compressAvailable == false then
            note = RED .. "This helper version can't compress textures (numpy is missing).|r"
            self.compress:SetEnabled(false)
        else
            self.compress:SetEnabled(true)
            if want ~= st.format then
                local target = want == "blp" and st.blp or st.tga
                note = YELLOW .. format("Click Send to apply: the helper converts your emotes (about %s -> %s); the 7TV bar shows when to reload.|r",
                    Bytes(st.bytes), Bytes(target))
            elseif (st.converting or 0) > 0 then
                note = ORANGE .. format("%d emotes switch format after you restart WoW.|r", st.converting)
            elseif want == "blp" then
                note = GREEN .. format("On. Uncompressed would use about %s.|r", Bytes(st.tga))
            else
                note = format("Compressing would bring this down to about %s.", Bytes(st.blp))
            end
        end
        self.compressNote:SetText(note)
    end
    return p
end

---------------------------------------------------------------------------
-- Window
---------------------------------------------------------------------------
local TABS = {
    { key = "emotes", label = "Emotes", build = BuildEmotesPage },
    { key = "add", label = "Add", build = BuildAddPage },
    { key = "downloads", label = "Downloads", build = BuildDownloadsPage },
    { key = "settings", label = "Settings", build = BuildSettingsPage },
}

function UI:Create()
    if self.frame then return end
    local f = CreateFrame("Frame", "SevenTVEmotesFrame", UIParent, "BackdropTemplate")
    self.frame = f
    f:SetSize(W, H)
    local pos = ns.db.uiPos
    if type(pos) == "table" and pos[1] then
        f:SetPoint(pos[1], UIParent, pos[2] or pos[1], pos[3] or 0, pos[4] or 0)
    else
        f:SetPoint("CENTER")
    end
    f:SetFrameStrata("HIGH")
    f:SetToplevel(true)
    f:SetClampedToScreen(true)
    f:EnableMouse(true)
    f:SetMovable(true)
    f:RegisterForDrag("LeftButton")
    f:SetScript("OnDragStart", f.StartMoving)
    f:SetScript("OnDragStop", function(s)
        s:StopMovingOrSizing()
        local p1, _, p2, x, y = s:GetPoint()
        ns.db.uiPos = { p1, p2, floor(x + 0.5), floor(y + 0.5) }
    end)
    Backdrop(f, BG, { ACCENT[1], ACCENT[2], ACCENT[3], 0.6 })
    if UISpecialFrames then table.insert(UISpecialFrames, "SevenTVEmotesFrame") end -- Esc closes it

    local title = Text(f, "GameFontNormalLarge", "|cff29b6f67TV|r Emotes")
    title:SetPoint("TOPLEFT", 14, -14)
    self.summary = Text(f, "GameFontHighlightSmall", "", "RIGHT")
    self.summary:SetPoint("TOPRIGHT", -44, -18)
    local close = Button(f, "X", 24, 24, function() f:Hide() end)
    close:SetPoint("TOPRIGHT", -10, -10)

    self.tabs, self.pages = {}, {}
    local content = CreateFrame("Frame", nil, f)
    content:SetPoint("TOPLEFT", 14, -82)
    content:SetPoint("BOTTOMRIGHT", -14, 50)
    local prev
    for _, t in ipairs(TABS) do
        local b = Button(f, t.label, 110, 26, function() UI:Show(t.key) end)
        b:SetPoint("TOPLEFT", prev or f, prev and "TOPRIGHT" or "TOPLEFT", prev and 4 or 14, prev and 0 or -44)
        self.tabs[t.key] = b
        prev = b
        local page = CreateFrame("Frame", nil, content)
        page:SetAllPoints()
        page:Hide()
        self.pages[t.key] = { frame = page, ctl = t.build(page) }
    end
    self.downloadsTab = self.tabs.downloads

    -- footer: what's waiting + reload
    local footer = CreateFrame("Frame", nil, f, "BackdropTemplate")
    footer:SetPoint("BOTTOMLEFT", 1, 1)
    footer:SetPoint("BOTTOMRIGHT", -1, 1)
    footer:SetHeight(40)
    Backdrop(footer, { 0.04, 0.04, 0.05, 1 }, { 0, 0, 0, 0 })
    self.footerText = Text(footer, "GameFontHighlight", "")
    self.footerText:SetPoint("LEFT", 14, 0)
    self.footerText:SetPoint("RIGHT", -170, 0)
    self.reload = Button(footer, "Reload", 150, 26, function() ns.ReloadUI() end, true)
    self.reload:SetPoint("RIGHT", -12, 0)

    f:SetScript("OnShow", function() UI:Refresh() end)
    f:SetScript("OnHide", function()
        for _, pg in pairs(UI.pages) do if pg.ctl.OnHide then pg.ctl:OnHide() end end
    end)
    table.insert(ns.queueListeners, function() if f:IsShown() then UI:Refresh() end end)
    f:Hide()
    self.current = "emotes"
end

function UI:Show(key)
    self:Create()
    self.current = key or self.current or "emotes"
    for k, pg in pairs(self.pages) do
        pg.frame:SetShown(k == self.current)
        local b = self.tabs[k]
        if k == self.current then
            b:SetBackdropColor(ACCENT[1] * 0.45, ACCENT[2] * 0.45, ACCENT[3] * 0.45, 1)
        else
            b:SetBackdropColor(0.13, 0.13, 0.16, 1)
        end
    end
    self.frame:Show()
    self:Refresh()
end

function UI:Toggle(key)
    self:Create()
    if self.frame:IsShown() and (not key or key == self.current) then
        self.frame:Hide()
    else
        self:Show(key)
    end
end

function UI:Refresh()
    if ns.StatusBar then ns.StatusBar:Refresh() end
    if not self.frame or not self.frame:IsShown() then return end
    local pg = self.pages[self.current]
    if pg and pg.ctl.Refresh then pg.ctl:Refresh() end

    local H = SevenTVEmotes_Helper
    local st = type(H) == "table" and type(H.storage) == "table" and H.storage or nil
    self.summary:SetText(format("%d emotes%s", ns.count, st and (" - " .. Bytes(st.bytes) .. " on disk") or ""))

    local unsent, sent = UI.QueueSummary()
    self.downloadsTab:SetLabel((unsent + sent) > 0 and format("Downloads (%d)", unsent + sent) or "Downloads")
    local wantFmt = ns.db.compress and "blp" or "tga"
    local fmtPending = st and st.format ~= wantFmt
    local compressChanged = ns.loadedCompress ~= nil and (ns.db.compress and true or false) ~= ns.loadedCompress
    if unsent > 0 or compressChanged then
        local parts = {}
        if unsent > 0 then parts[#parts + 1] = format("%d request%s waiting to be sent", unsent, unsent == 1 and "" or "s") end
        if compressChanged then parts[#parts + 1] = "compression setting changed" end
        self.footerText:SetText(YELLOW .. table.concat(parts, ", ") .. ".|r Send now, or they go out when you log out.")
        self.reload:SetLabel("Send to helper")
        self.reload:Show()
    elseif sent > 0 or fmtPending then
        self.footerText:SetText(format("The helper is working%s. The 7TV bar by your chat shows when it's ready.",
            sent > 0 and format(" on %d request%s", sent, sent == 1 and "" or "s") or ""))
        self.reload:SetLabel("Reload")
        self.reload:Show()
    else
        self.footerText:SetText(GREY .. "Type /7tv help for chat commands.|r")
        self.reload:Hide()
    end
end
