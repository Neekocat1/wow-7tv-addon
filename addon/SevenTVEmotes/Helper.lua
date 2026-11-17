-- SevenTVEmotes - Helper bridge
-- The in-game side of the 7TV Emotes Helper app.
--
-- WoW addons can't download files, so new emotes come from a small app on the player's
-- PC. The two sides talk through files:
--   addon -> helper : requests in SevenTVEmotesDB.queue (saved to disk on /reload or logout)
--   helper -> addon : SevenTVEmotes_Local/Local.lua (read when the UI loads), which holds
--                     the emotes plus SevenTVEmotes_Helper = { version, heartbeat, results, ... }
local ADDON_NAME, ns = ...
local format = string.format

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

ns.sessionIds = {}       -- requests queued since the last reload (not on disk yet)
ns.queueListeners = {}

function ns.NotifyQueueChanged(reason)
    for _, fn in ipairs(ns.queueListeners) do fn(reason) end
end

-- kind: "add" | "remove" | "channel" | "unchannel"
-- Returns ok, message. With `quiet`, no popup (the /7tv window shows its own Reload button).
function ns.QueueRequest(kind, ref, alias, label, quiet)
    local state = ns.HelperState()
    if state == "missing" then
        if not quiet then ns.Print(MISSING_HELP) end
        return false, MISSING_HELP
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
            ns.Print(MISSING_HELP)
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
            ns.Print("No emotes installed. " .. MISSING_HELP)
        else
            ns.Print("No emotes yet. Try /7tv add <7tv.app emote link> or /7tv channel <twitch name>.")
        end
    end
end
