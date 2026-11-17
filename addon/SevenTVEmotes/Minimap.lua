-- SevenTVEmotes - minimap button
-- Left-click toggles the /7tv window, right-click opens its Settings tab, drag to move it
-- around the minimap. Position and visibility are saved (minimapAngle, minimap).
-- Also hooks into the minimap's addon-compartment menu on clients that have it (see the .toc).
local ADDON_NAME, ns = ...

local cos, sin, rad, deg, sqrt, max = math.cos, math.sin, math.rad, math.deg, math.sqrt, math.max
local atan2 = math.atan2 or function(y, x) return math.atan(y, x) end
local format = string.format

local MM = {}
ns.Minimap = MM

local ICON = "Interface\\AddOns\\" .. ADDON_NAME .. "\\media\\minimap"

-- Offset from the minimap's centre for an angle in degrees (0 = right, 90 = top).
-- Round minimaps: on the circle. Square ones (GetMinimapShape() == "SQUARE"): pushed out to the
-- edge of the square, so the button hugs the border instead of floating inside it.
function MM.Offset(angle, radius, shape)
    local a = rad(angle)
    local x, y = cos(a), sin(a)
    if shape == "SQUARE" then
        local m = max(math.abs(x), math.abs(y))
        x, y = x / m, y / m
        -- keep the corners from poking out too far
        local len = sqrt(x * x + y * y)
        if len > 1.3 then x, y = x * 1.3 / len, y * 1.3 / len end
    end
    return x * radius, y * radius
end

local function Shape()
    return GetMinimapShape and GetMinimapShape() or "ROUND"
end

function MM:Place()
    local b, map = self.button, self.map
    if not b then return end
    local radius = (map:GetWidth() or 140) / 2 + 5
    local x, y = MM.Offset(ns.db.minimapAngle or 225, radius, Shape())
    b:ClearAllPoints()
    b:SetPoint("CENTER", map, "CENTER", x, y)
end

function MM:Update()
    if not self.button then return end
    self.button:SetShown(ns.db.minimap ~= false)
    self:Place()
end

-- Angle of the cursor around the minimap's centre, in degrees [0, 360).
function MM:CursorAngle()
    local mx, my = self.map:GetCenter()
    local px, py = GetCursorPosition()
    local scale = self.map:GetEffectiveScale()
    return deg(atan2(py / scale - my, px / scale - mx)) % 360
end

local function OnDragUpdate(self)
    ns.db.minimapAngle = MM:CursorAngle()
    MM:Place()
end

function MM:Tooltip(tip)
    tip:AddLine("|cff29b6f67TV|r Emotes")
    tip:AddLine(format("%d emotes", ns.count or 0), 1, 1, 1)
    local state = ns.HelperState and ns.HelperState() or "missing"
    if state == "ok" then
        tip:AddLine("Helper app running", 0.25, 1, 0.25)
    elseif state == "stale" then
        tip:AddLine("Helper app not seen recently", 1, 0.82, 0)
    else
        tip:AddLine("Helper app not installed", 1, 0.38, 0.38)
    end
    local queued = ns.db.queue and #ns.db.queue or 0
    if queued > 0 then
        tip:AddLine(format("%d request%s waiting for the helper", queued, queued == 1 and "" or "s"), 1, 0.82, 0)
    end
    tip:AddLine(" ")
    tip:AddLine("Left-click: open the 7TV window", 0.6, 0.6, 0.6)
    tip:AddLine("Right-click: settings", 0.6, 0.6, 0.6)
    if self.button then tip:AddLine("Drag: move this button", 0.6, 0.6, 0.6) end
end

function MM:Click(button)
    if not ns.UI then return end
    if button == "RightButton" then
        ns.UI:Toggle("settings")
    else
        ns.UI:Toggle()
    end
end

function MM:Create()
    if self.button then return self.button end
    local map = Minimap
    if not map then return end
    self.map = map

    local b = CreateFrame("Button", "SevenTVEmotesMinimapButton", map)
    self.button = b
    b:SetSize(31, 31)
    b:SetFrameStrata("MEDIUM")
    b:SetFrameLevel((map:GetFrameLevel() or 1) + 8)
    b:RegisterForClicks("LeftButtonUp", "RightButtonUp")
    b:RegisterForDrag("LeftButton")
    b:SetHighlightTexture("Interface\\Minimap\\UI-Minimap-ZoomButton-Highlight")

    -- Same layering as the standard round minimap buttons.
    local bg = b:CreateTexture(nil, "BACKGROUND")
    bg:SetSize(20, 20)
    bg:SetTexture("Interface\\Minimap\\UI-Minimap-Background")
    bg:SetPoint("TOPLEFT", 7, -5)
    local icon = b:CreateTexture(nil, "ARTWORK")
    icon:SetSize(19, 19)
    icon:SetTexture(ICON)
    icon:SetPoint("TOPLEFT", 6, -6)
    b.icon = icon
    local border = b:CreateTexture(nil, "OVERLAY")
    border:SetSize(53, 53)
    border:SetTexture("Interface\\Minimap\\MiniMap-TrackingBorder")
    border:SetPoint("TOPLEFT")

    b:SetScript("OnClick", function(_, button) MM:Click(button) end)
    b:SetScript("OnMouseDown", function() icon:SetPoint("TOPLEFT", 7, -7) end)
    b:SetScript("OnMouseUp", function() icon:SetPoint("TOPLEFT", 6, -6) end)
    b:SetScript("OnDragStart", function(self)
        self:LockHighlight()
        self:SetScript("OnUpdate", OnDragUpdate)
        GameTooltip:Hide()
    end)
    b:SetScript("OnDragStop", function(self)
        self:SetScript("OnUpdate", nil)
        self:UnlockHighlight()
        icon:SetPoint("TOPLEFT", 6, -6)
    end)
    b:SetScript("OnEnter", function(self)
        GameTooltip:SetOwner(self, "ANCHOR_LEFT")
        MM:Tooltip(GameTooltip)
        GameTooltip:Show()
    end)
    b:SetScript("OnLeave", function() GameTooltip:Hide() end)

    self:Update()
    return b
end

function MM:SetShown(show)
    ns.db.minimap = show and true or false
    self:Create()
    self:Update()
end

-- Addon compartment (the addon list button on the minimap, Dragonflight and later). The .toc
-- names these globals; clients without the compartment simply ignore those lines.
function SevenTVEmotes_OnAddonCompartmentClick(_, button)
    MM:Click(button)
end

function SevenTVEmotes_OnAddonCompartmentEnter(_, frame)
    GameTooltip:SetOwner(frame or UIParent, "ANCHOR_LEFT")
    MM:Tooltip(GameTooltip)
    GameTooltip:Show()
end

function SevenTVEmotes_OnAddonCompartmentLeave()
    GameTooltip:Hide()
end

local events = CreateFrame("Frame")
events:RegisterEvent("PLAYER_LOGIN")
events:SetScript("OnEvent", function()
    MM:Create()
end)
