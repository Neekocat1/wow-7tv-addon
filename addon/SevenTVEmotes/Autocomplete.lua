-- SevenTVEmotes - Autocomplete
-- Type ":pee" in any chat box to get suggestions.
--   Tab / Shift-Tab  fill in and cycle through suggestions
--   type ":" again   accept the highlighted suggestion (":pee:" -> "peepoHappy ")
--   click a row      accept it
-- Only post-hooks (HookScript) are used, so Blizzard's chat handlers stay untainted.
local ADDON_NAME, ns = ...

local issecret = ns.issecret
local MAX_ROWS = 10
local ROW_H = 26
local ICON_H = 22

local popup, rows
local st = { eb = nil, start = 0, stop = 0, matches = {}, sel = 1, cycling = false }

local function Hide()
    if popup then popup:Hide() end
    st.eb, st.cycling = nil, false
    wipe(st.matches)
end

local function FindMatches(q, out)
    wipe(out)
    local lq = q:lower()
    local contains = {}
    local names, lnames, emotes, disabled = ns.names, ns.lnames, ns.emotes, ns.db.disabled
    for i = 1, #names do
        local p = lnames[i]:find(lq, 1, true)
        if p and not disabled[names[i]] then
            if p == 1 then
                out[#out + 1] = emotes[names[i]]
                if #out >= MAX_ROWS then return end
            elseif #contains < MAX_ROWS then
                contains[#contains + 1] = emotes[names[i]]
            end
        end
    end
    for i = 1, #contains do
        if #out >= MAX_ROWS then return end
        out[#out + 1] = contains[i]
    end
end

local function ReplaceRange(eb, s, e, insert)
    local text = eb:GetText()
    eb:SetText(text:sub(1, s - 1) .. insert .. text:sub(e + 1))
    eb:SetCursorPosition(s - 1 + #insert)
end

local function Accept(addSpace)
    local eb, e = st.eb, st.matches[st.sel]
    if not eb or not e then return Hide() end
    ReplaceRange(eb, st.start, st.stop, e.name .. (addSpace and " " or ""))
    Hide()
    if eb:IsShown() then eb:SetFocus() end
end

local function RefreshRows()
    local width = 160
    for i = 1, MAX_ROWS do
        local row, e = rows[i], st.matches[i]
        if e then
            row.index = i
            local w = math.min(math.floor(ICON_H * e.w / e.h + 0.5), ICON_H * 3)
            row.icon:SetSize(w, ICON_H)
            row.icon:SetTexture(e.path)
            local l, r, t, b = ns.Coords(e, e.frames and 0 or nil)
            row.icon:SetTexCoord(l / e.tw, r / e.tw, t / e.th, b / e.th)
            ns.animTextures[row.icon] = e.frames and e or nil
            row.text:SetText(e.name)
            row.sel:SetShown(i == st.sel)
            width = math.max(width, w + row.text:GetStringWidth() + 24)
            row:Show()
        else
            ns.animTextures[row.icon] = nil
            row:Hide()
        end
    end
    popup:SetSize(math.min(width, 400), #st.matches * ROW_H + 8)
end

local function CreatePopup()
    popup = CreateFrame("Frame", "SevenTVEmotesAutocomplete", UIParent, BackdropTemplateMixin and "BackdropTemplate" or nil)
    popup:SetFrameStrata("DIALOG")
    popup:SetClampedToScreen(true)
    popup:Hide()
    if popup.SetBackdrop then
        popup:SetBackdrop({ bgFile = "Interface\\Buttons\\WHITE8X8", edgeFile = "Interface\\Buttons\\WHITE8X8", edgeSize = 1 })
        popup:SetBackdropColor(0.05, 0.05, 0.07, 0.94)
        popup:SetBackdropBorderColor(0.16, 0.71, 0.96, 0.8)
    end
    rows = {}
    for i = 1, MAX_ROWS do
        local row = CreateFrame("Button", nil, popup)
        row:SetHeight(ROW_H)
        row:SetPoint("TOPLEFT", popup, "TOPLEFT", 4, -4 - (i - 1) * ROW_H)
        row:SetPoint("RIGHT", popup, "RIGHT", -4, 0)

        row.sel = row:CreateTexture(nil, "BACKGROUND")
        row.sel:SetAllPoints()
        row.sel:SetColorTexture(0.16, 0.71, 0.96, 0.25)

        local hl = row:CreateTexture(nil, "HIGHLIGHT")
        hl:SetAllPoints()
        hl:SetColorTexture(1, 1, 1, 0.08)

        row.icon = row:CreateTexture(nil, "ARTWORK")
        row.icon:SetPoint("LEFT", row, "LEFT", 2, 0)

        row.text = row:CreateFontString(nil, "OVERLAY", "GameFontHighlight")
        row.text:SetPoint("LEFT", row.icon, "RIGHT", 6, 0)
        row.text:SetJustifyH("LEFT")

        row:SetScript("OnClick", function(self)
            st.sel = self.index
            Accept(true)
        end)
        rows[i] = row
    end
end

local function Show(eb)
    if not popup then CreatePopup() end
    popup:ClearAllPoints()
    popup:SetPoint("BOTTOMLEFT", eb, "TOPLEFT", 0, 4)
    RefreshRows()
    popup:Show()
end

local function OnTextChanged(eb, userInput)
    if not userInput then return end -- our own SetText calls
    local db = ns.db
    if not (db.enabled and db.autocomplete) or ns.count == 0 then return Hide() end
    local text = eb:GetText()
    if issecret(text) or not text then return Hide() end
    local cursor = eb:GetCursorPosition()

    if st.cycling and st.eb == eb then
        -- Typing ":" right after a Tab-filled name accepts it with a space.
        if cursor == st.stop + 1 and text:sub(cursor, cursor) == ":" then
            ReplaceRange(eb, cursor, cursor, " ")
            return Hide()
        end
        Hide()
    end

    local before = text:sub(1, cursor)

    -- ":query:" accepts the highlighted suggestion.
    local s = before:match("()%:[^%s:]+:$")
    if s and st.eb == eb and popup and popup:IsShown() and s == st.start then
        st.stop = cursor
        return Accept(true)
    end

    local q
    s, q = before:match("()%:([^%s:]+)$")
    if not s or #q < (db.minChars or 2) or (s > 1 and not before:sub(s - 1, s - 1):find("%s")) then
        return Hide()
    end
    FindMatches(q, st.matches)
    if #st.matches == 0 then return Hide() end
    st.eb, st.start, st.stop, st.sel, st.cycling = eb, s, cursor, 1, false
    Show(eb)
end

local function OnTabPressed(eb)
    if not popup or not popup:IsShown() or st.eb ~= eb or #st.matches == 0 then return end
    if st.cycling then
        st.sel = st.sel + (IsShiftKeyDown() and -1 or 1)
        if st.sel > #st.matches then st.sel = 1 elseif st.sel < 1 then st.sel = #st.matches end
    end
    local e = st.matches[st.sel]
    ReplaceRange(eb, st.start, st.stop, e.name)
    st.stop = st.start + #e.name - 1
    st.cycling = true
    RefreshRows()
end

local function DelayedHide(eb)
    C_Timer.After(0.2, function()
        if st.eb == eb and not eb:HasFocus() then Hide() end
    end)
end

local hooked = {}
local function HookEditBox(eb)
    if not eb or hooked[eb] then return end
    hooked[eb] = true
    eb:HookScript("OnTextChanged", OnTextChanged)
    eb:HookScript("OnTabPressed", OnTabPressed)
    eb:HookScript("OnEnterPressed", Hide)
    eb:HookScript("OnEscapePressed", Hide)
    eb:HookScript("OnEditFocusLost", DelayedHide)
    eb:HookScript("OnHide", function(self) if st.eb == self then Hide() end end)
end

function ns.SetupAutocomplete()
    ns.OnChatFrame(function(cf)
        HookEditBox(cf.editBox or _G[(cf:GetName() or "") .. "EditBox"])
    end)
end
