-- SevenTVEmotes - Helper bridge
-- The in-game side of the 7TV Emotes Helper app.
--
-- WoW addons can't download files, so new emotes come from a small app on the player's
-- PC. The two sides talk through files:
--   addon -> helper : requests in SevenTVEmotesDB.queue (saved to disk on /reload or logout)
--   helper -> addon : SevenTVEmotes_Local/Local.lua (read when the UI loads), which holds
--                     the emotes plus SevenTVEmotes_Helper = { version, heartbeat, results, ... }
-- Players without the helper can instead download an emote pack from the web pack builder: the
-- same folder, with SevenTVEmotes_Pack = { built, url, channels, ... } in place of the helper's table.
local ADDON_NAME, ns = ...
local format = string.format
local date = date or (os and os.date)

ns.PACK_URL = "https://neekocat1.github.io/wow-7tv-addon/"

local STALE_AFTER = 15 * 60 -- the helper rewrites Local.lua every few minutes while running

-- "ok" | "stale" | "missing", helperTable
function ns.HelperState()
    local H = SevenTVEmotes_Helper
    if type(H) ~= "table" or not H.version then return "missing" end
    if time() - (H.heartbeat or 0) > STALE_AFTER then return "stale", H end
    return "ok", H
end

local function Reload()
    if C_UI and C_UI.Reload then C_UI.Reload() else ReloadUI() end
end

local MISSING_HELP = "Adding emotes needs the free 7TV Emotes Helper app running on your PC. "
    .. "Install it once and it starts with Windows."

-- The downloaded emote pack's info, when the emotes come from a pack rather than the helper.
function ns.PackInfo()
    local P = SevenTVEmotes_Pack
    if type(P) ~= "table" or type(SevenTVEmotes_Helper) == "table" then return nil end
    return P
end

function ns.PackUrl()
    local P = ns.PackInfo()
    return (P and type(P.url) == "string" and P.url ~= "" and P.url) or ns.PACK_URL
end

-- "Downloaded emote pack from 2026-11-17 (grimmybear, 7TV globals)"
function ns.PackSummary(P)
    P = P or ns.PackInfo()
    if not P then return "" end
    local from = {}
    for _, c in ipairs(type(P.channels) == "table" and P.channels or {}) do from[#from + 1] = tostring(c) end
    if P.global then from[#from + 1] = "7TV globals" end
    for _, s in ipairs(type(P.sets) == "table" and P.sets or {}) do from[#from + 1] = tostring(s) end
    local when = (tonumber(P.built) and date) and date("%Y-%m-%d", tonumber(P.built)) or "earlier"
    return format("Downloaded emote pack from %s%s", when, #from > 0 and (" (" .. table.concat(from, ", ") .. ")") or "")
end

-- What to tell a player who has no helper.
function ns.MissingHelp()
    if ns.PackInfo() then
        return "Your emotes come from a downloaded emote pack. To change them, build a new pack at "
            .. ns.PackUrl() .. " and restart WoW, or install the free 7TV Emotes Helper app to add emotes in game."
    end
    return MISSING_HELP .. " Or download a ready-made emote pack at " .. ns.PACK_URL
end

ns.sessionIds = {}       -- requests queued since the last reload (not on disk yet)
ns.queueListeners = {}

function ns.NotifyQueueChanged(reason)
    for _, fn in ipairs(ns.queueListeners) do fn(reason) end
end

-- kind: "add" | "remove" | "channel" | "unchannel" | "order" (channel priority, see below)
-- Returns ok, message. With `quiet`, no popup (the /7tv window shows its own Reload button).
function ns.QueueRequest(kind, ref, alias, label, quiet)
    local state = ns.HelperState()
    if state == "missing" then
        if not quiet then ns.Print(ns.MissingHelp()) end
        return false, ns.MissingHelp()
    end
    local q = ns.db.queue
    for _, r in ipairs(q) do
        if r.kind == kind and r.ref == ref and r.alias == alias then
            if not quiet then ns.Print(label .. " is already queued.") end
            return false, label .. " is already queued."
        end
    end
    local id = format("%d-%04d", time(), math.random(0, 9999))
    q[#q + 1] = { id = id, kind = kind, ref = ref, alias = alias, t = time() }
    ns.sessionIds[id] = true
    ns.NotifyQueueChanged("added")
    local msg = label .. " queued. Click Send on the 7TV bar when you're ready (logging out sends it too)."
    if state == "stale" then
        msg = msg .. " |cffffd100The helper app hasn't checked in recently; make sure it's running.|r"
    end
    if not quiet then ns.Print(msg) end
    return true, msg
end

function ns.CancelRequest(id)
    local q = ns.db.queue
    for i = #q, 1, -1 do
        if q[i].id == id then table.remove(q, i) end
    end
    ns.sessionIds[id] = nil
    ns.NotifyQueueChanged()
end

-- "0.10.2" >= "0.7.0"?
local function AtLeast(version, want)
    local a, b = {}, {}
    for n in tostring(version):gmatch("%d+") do a[#a + 1] = tonumber(n) end
    for n in tostring(want):gmatch("%d+") do b[#b + 1] = tonumber(n) end
    for i = 1, math.max(#a, #b) do
        local x, y = a[i] or 0, b[i] or 0
        if x ~= y then return x > y end
    end
    return true
end

-- Requests newer than the first helper release, with the helper version that understands them
-- (an older helper answers "Unknown request").
local NEEDS = { order = "0.7.0" }

function ns.HelperSupports(kind)
    local _, H = ns.HelperState()
    return H ~= nil and AtLeast(H.version, NEEDS[kind] or "0")
end

-- Channel priority -------------------------------------------------------------
-- When two channels have an emote with the same name, the one higher in the player's list wins.
-- The helper keeps the list (SevenTVEmotes_Helper.channels, highest first); a reorder is sent as
-- an "order" request whose ref is the logins, highest first, separated by commas.

local function Split(s)
    local out = {}
    for login in tostring(s or ""):gmatch("[^,%s]+") do out[#out + 1] = login end
    return out
end

-- The list as the player sees it: the helper's order with queued changes applied. New channels
-- go on top, newest first (as the helper does); a queued reorder applies to the channels it
-- names, and channels it doesn't name stay on top.
-- Returns { { login = , queued = true if not added yet, removing = true if removal queued }, ... }
function ns.ChannelList()
    local _, H = ns.HelperState()
    local order, adds, removing = nil, {}, {}
    for _, r in ipairs(ns.db.queue) do
        if r.kind == "order" then order = Split(r.ref) end
        if r.kind == "channel" then adds[#adds + 1] = r.ref end
        if r.kind == "unchannel" then removing[r.ref] = true end
    end
    local list, seen = {}, {}
    local have = {}
    if H and type(H.channels) == "table" then
        for _, login in ipairs(H.channels) do have[login] = true end
    end
    for i = #adds, 1, -1 do
        local login = adds[i]
        if not have[login] and not seen[login] then
            seen[login] = true
            list[#list + 1] = { login = login, queued = true }
        end
    end
    if H and type(H.channels) == "table" then
        for _, login in ipairs(H.channels) do
            if not seen[login] then
                seen[login] = true
                list[#list + 1] = { login = login, removing = removing[login] }
            end
        end
    end
    if order then
        local byLogin, named, rest = {}, {}, {}
        for _, row in ipairs(list) do byLogin[row.login] = row end
        local inOrder = {}
        for _, login in ipairs(order) do
            if byLogin[login] and not inOrder[login] then
                inOrder[login] = true
                named[#named + 1] = byLogin[login]
            end
        end
        for _, row in ipairs(list) do
            if not inOrder[row.login] then rest[#rest + 1] = row end
        end
        list = rest
        for _, row in ipairs(named) do list[#list + 1] = row end
    end
    return list
end

-- The queued reorder, if there is one.
function ns.QueuedOrder()
    for _, r in ipairs(ns.db.queue) do
        if r.kind == "order" then return r end
    end
end

-- Move a channel `delta` places (-1 = up = higher priority). Queues one "order" request that
-- replaces any earlier one; moving back to the helper's own order cancels it.
function ns.MoveChannel(login, delta)
    local list = ns.ChannelList()
    local i
    for k, row in ipairs(list) do
        if row.login == login then i = k end
    end
    local j = i and i + delta
    if not j or j < 1 or j > #list then return false end
    list[i], list[j] = list[j], list[i]
    local logins = {}
    for k, row in ipairs(list) do logins[k] = row.login end
    local old = ns.QueuedOrder()
    if old then
        local q = ns.db.queue
        for k = #q, 1, -1 do
            if q[k].kind == "order" then table.remove(q, k) end
        end
        ns.sessionIds[old.id] = nil
    end
    local _, H = ns.HelperState()
    local current = H and type(H.channels) == "table" and table.concat(H.channels, ",") or ""
    if table.concat(logins, ",") == current then
        ns.NotifyQueueChanged()
        return true
    end
    return ns.QueueRequest("order", table.concat(logins, ","), nil, "New channel priority", true)
end

function ns.ReloadUI() Reload() end

-- WoW can only load texture files that existed when the game started. Ask the client about
-- each helper-made emote; SetTexture reports false for files it can't see (on clients that
-- return a result at all), which means WoW needs a restart before they show.
function ns.ProbeTextures()
    ns.invisible = {}
    local probe = ns.probeTexture
    if not probe then
        local f = CreateFrame("Frame")
        f:Hide()
        probe = f:CreateTexture()
        ns.probeTexture = probe
    end
    for _, name in ipairs(ns.names) do
        local e = ns.emotes[name]
        if e.pack == "My emotes" then
            local ok = probe:SetTexture(e.path)
            if ok == false then ns.invisible[#ns.invisible + 1] = name end
        end
    end
    -- Tell the helper (written to disk on the next reload/logout); it logs this for debugging.
    local files = {}
    for i = 1, math.min(#ns.invisible, 50) do files[i] = ns.emotes[ns.invisible[i]].path end
    ns.db.invisible = files
    return #ns.invisible
end

function ns.OnLoginHelper()
    local state, H = ns.HelperState()
    local missing = ns.ProbeTextures()
    if missing > 0 then
        ns.Print(format("%d emote%s can't be shown until you restart WoW (their files were created after the game started).",
            missing, missing == 1 and "" or "s"))
    end
    local q = ns.db.queue

    -- Report what the helper did with earlier requests and drop them from the queue.
    if H and type(H.results) == "table" then
        local remaining = {}
        for _, r in ipairs(q) do
            local res = H.results[r.id]
            if res then
                local hist = ns.db.history
                table.insert(hist, 1, { ok = res.ok and true or false, msg = tostring(res.msg), name = res.name,
                    kind = r.kind, ref = r.ref, t = time() })
                for i = #hist, 51, -1 do hist[i] = nil end
                local e = res.ok and res.name and ns.emotes[res.name]
                local icon = e and (ns.Tag(e, 20, e.frames and 0 or nil) .. " ") or ""
                ns.Print(icon .. (res.ok and "|cff40ff40" or "|cffff6060") .. tostring(res.msg) .. "|r")
            else
                remaining[#remaining + 1] = r
            end
        end
        for i = #q, 1, -1 do q[i] = nil end
        for i, r in ipairs(remaining) do q[i] = r end
    end

    if #q > 0 and state ~= "ok" then
        if state == "stale" then
            ns.Print(format("%d request%s waiting for the helper app. Start it (it may have been closed), then /reload.",
                #q, #q == 1 and " is" or "s are"))
        else
            ns.Print(ns.MissingHelp())
        end
    end
    -- The status bar shows the helper's live progress while requests are being worked on.
    if C_Timer and C_Timer.After then
        C_Timer.After(1, function() if ns.StatusBar then ns.StatusBar:Refresh() end end)
    elseif ns.StatusBar then
        ns.StatusBar:Refresh()
    end

    if H and type(H.notice) == "string" and H.notice ~= "" then ns.Print(H.notice) end

    if ns.count == 0 then
        if state == "missing" then
            ns.Print("No emotes installed. " .. ns.MissingHelp())
        else
            ns.Print("No emotes yet. Try /7tv add <7tv.app emote link> or /7tv channel <twitch name>.")
        end
    end
end
