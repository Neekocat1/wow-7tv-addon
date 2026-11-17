-- SevenTVEmotes - Slash commands
local ADDON_NAME, ns = ...
local floor, format = math.floor, string.format
local OnOff = ns.OnOff

local function PrintEmoteList(list)
    local line, n = {}, 0
    for _, e in ipairs(list) do
        n = n + 1
        line[#line + 1] = ns.Tag(e, 20, e.frames and 0 or nil) .. " " .. e.name
        if n % 5 == 0 then
            ns.Print(table.concat(line, "   "))
            line = {}
        end
    end
    if #line > 0 then ns.Print(table.concat(line, "   ")) end
end

local function SearchEmotes(q, limit)
    local out, lq = {}, (q or ""):lower()
    for i, name in ipairs(ns.names) do
        if lq == "" or ns.lnames[i]:find(lq, 1, true) then
            out[#out + 1] = ns.emotes[name]
            if #out >= limit then break end
        end
    end
    return out
end

local function Toggle(key, arg, label)
    if arg == "on" then ns.db[key] = true
    elseif arg == "off" then ns.db[key] = false
    else ns.db[key] = not ns.db[key] end
    ns.Print(label .. " " .. OnOff(ns.db[key]))
end

local function ShortRef(ref)
    if ref:find("7tv", 1, true) or #ref >= 24 then return "Emote" end
    return ref
end

local function HelperStatus()
    local state, H = ns.HelperState()
    if state == "missing" then
        ns.Print("Helper app: |cffff6060not installed|r. Without it you can use emotes but not add new ones.")
        return
    end
    local mins = floor((time() - (H.heartbeat or 0)) / 60)
    ns.Print(format("Helper app %s: %s (last check-in %d min before this login).",
        tostring(H.version), state == "ok" and "|cff40ff40running|r" or "|cffffd100not seen recently|r", mins))
    if type(H.channels) == "table" and #H.channels > 0 then
        ns.Print("Channels: " .. table.concat(H.channels, ", "))
    end
    if H.slotsFree then
        ns.Print(format("Room for %d more emotes before the next WoW restart.", H.slotsFree))
    end
end

local HELP = {
    "/7tv - open the 7TV Emotes window (also: /7tv emotes | add | downloads | settings)",
    "|cffffd100Adding emotes|r (uses the helper app)",
    "  /7tv add <7tv link or name> [alias] - add one emote",
    "  /7tv channel <twitch name> - add a streamer's 7TV emotes",
    "  /7tv unchannel <twitch name> - remove them again",
    "  /7tv remove <name> - remove an emote",
    "  /7tv queue - show requests waiting for the helper",
    "  /7tv helper - helper app status",
    "  /7tv status - show the live status bar (drag to move, x to hide)",
    "  /7tv texturetest - check which texture formats WoW loads",
    "|cffffd100Using emotes|r",
    "  /7tv search <text> - list matching emotes",
    "  /7tv config - settings panel",
    "  /7tv size <12-64> - emote height in chat",
    "  /7tv on | off, anim on | off, complete on | off, tips on | off",
    "  /7tv minimap [on | off] - show/hide the minimap button",
    "  /7tv disable <name> | enable <name> - hide/show one emote",
}

local function Slash(input)
    local cmd, rest = (input or ""):match("^%s*(%S*)%s*(.-)%s*$")
    cmd = (cmd or ""):lower()
    if cmd == "" then
        ns.UI:Toggle()
    elseif (cmd == "add" and rest == "") or cmd == "emotes" or cmd == "downloads" or cmd == "settings" then
        ns.UI:Show(cmd)
    elseif cmd == "help" then
        ns.Print(format("%d emotes. Emotes %s, animation %s, autocomplete %s.",
            ns.count, OnOff(ns.db.enabled), OnOff(ns.db.animate), OnOff(ns.db.autocomplete)))
        for _, l in ipairs(HELP) do ns.Print(l) end

    elseif cmd == "add" then
        local ref, alias = rest:match("^(%S+)%s*(%S*)")
        if not ref then ns.Print("Usage: /7tv add <7tv.app emote link or emote name> [alias]") return end
        if alias == "" then alias = nil end
        if alias and alias:find("|", 1, true) then ns.Print("Aliases can't contain |.") return end
        ns.QueueRequest("add", ref, alias, alias or ShortRef(ref))

    elseif cmd == "remove" or cmd == "delete" then
        local e = ns.emotes[rest]
        if not e then ns.Print("No emote named '" .. rest .. "' (names are case-sensitive).") return end
        if e.pack == "Bundled" then
            ns.Print("That emote ships with the addon; use /7tv disable " .. rest .. " to hide it.")
            return
        end
        ns.QueueRequest("remove", rest, nil, "Removing " .. rest)

    elseif cmd == "channel" or cmd == "unchannel" then
        local login = rest:match("^@?(%S+)")
        if not login then ns.Print("Usage: /7tv " .. cmd .. " <twitch name>") return end
        login = login:lower()
        ns.QueueRequest(cmd, login, nil, (cmd == "channel" and "Channel " or "Removing channel ") .. login)

    elseif cmd == "queue" then
        local q = ns.db.queue
        if rest == "clear" then
            for i = #q, 1, -1 do q[i] = nil end
            ns.Print("Queue cleared.")
        elseif #q == 0 then
            ns.Print("Nothing waiting for the helper.")
        else
            for _, r in ipairs(q) do
                ns.Print(format("  %s %s%s", r.kind, r.ref, r.alias and (" as " .. r.alias) or ""))
            end
            ns.Print("Reload to send these to the helper. /7tv queue clear to cancel.")
        end

    elseif cmd == "helper" then
        HelperStatus()

    elseif cmd == "texturetest" or cmd == "test" then
        ns.ShowTextureTest()

    elseif cmd == "status" then
        ns.StatusBar:ShowManual()
        if not (ns.Board and ns.Board()) then
            ns.Print("The live status needs helper 0.4 or newer, and WoW must be restarted once after the helper creates its status files.")
        else
            ns.Print("Live status shown above your chat. The time on it should tick every few seconds without a reload; if it says 'Waiting for the helper app' for long, restart WoW once.")
        end

    elseif cmd == "config" or cmd == "options" then
        ns.OpenSettings()

    elseif cmd == "search" or cmd == "list" or cmd == "find" then
        local list = SearchEmotes(rest, 40)
        if #list == 0 then ns.Print("No emotes match '" .. rest .. "'.") return end
        PrintEmoteList(list)
        if #list == 40 then ns.Print("(showing the first 40 - narrow the search)") end

    elseif cmd == "size" then
        local n = tonumber(rest)
        if n and n >= 12 and n <= 64 then
            ns.db.size = floor(n)
            ns.Print("Emote size set to " .. ns.db.size .. " (applies to new messages).")
        else
            ns.Print("Usage: /7tv size <12-64>. Current: " .. ns.db.size)
        end

    elseif cmd == "on" or cmd == "off" then
        Toggle("enabled", cmd, "Emotes")
    elseif cmd == "anim" or cmd == "animate" then
        Toggle("animate", rest:lower(), "Animation")
    elseif cmd == "complete" or cmd == "autocomplete" then
        Toggle("autocomplete", rest:lower(), "Autocomplete")
    elseif cmd == "tips" or cmd == "links" then
        Toggle("links", rest:lower(), "Hover tooltips (new messages)")
    elseif cmd == "minimap" or cmd == "icon" then
        Toggle("minimap", rest:lower(), "Minimap button")
        if ns.Minimap then ns.Minimap:SetShown(ns.db.minimap) end

    elseif cmd == "disable" or cmd == "enable" then
        local e = ns.emotes[rest]
        if not e then ns.Print("No emote named '" .. rest .. "' (names are case-sensitive).") return end
        ns.db.disabled[rest] = (cmd == "disable") or nil
        ns.Print(ns.Tag(e, 20) .. " " .. rest .. " " .. (cmd == "disable" and "hidden" or "shown") .. ".")

    elseif cmd == "debug" then
        local state, H = ns.HelperState()
        ns.Print(format("Addon %s, helper %s (%s), %d emotes, %d packs.",
            C_AddOns and C_AddOns.GetAddOnMetadata and C_AddOns.GetAddOnMetadata(ADDON_NAME, "Version") or "?",
            H and tostring(H.version) or "none", state, ns.count, #ns.packs))
        local loaded = C_AddOns and C_AddOns.IsAddOnLoaded and C_AddOns.IsAddOnLoaded("SevenTVEmotes_Local")
        ns.Print("SevenTVEmotes_Local loaded: " .. tostring(loaded))
        if H and type(H.storage) == "table" then
            ns.Print(format("Texture format: %s (setting: %s), converting: %s",
                tostring(H.storage.format), ns.db.compress and "compressed" or "uncompressed", tostring(H.storage.converting)))
        end
        local n = ns.ProbeTextures()
        ns.Print(format("Emote files WoW can't see yet: %d", n))
        local e = ns.emotes[ns.names[1] or ""]
        if e then
            ns.Print("First emote: " .. e.name .. " -> " .. e.path)
            ns.Print("Looks like: " .. ns.Tag(e, 32, e.frames and 0 or nil))
        end
        if H and H.notice then ns.Print("Helper says: " .. H.notice) end
    elseif cmd == "info" or cmd == "packs" then
        for _, p in ipairs(ns.packs) do
            ns.Print(format("%s: %d emotes (%d animated)", p.name, p.count, p.animated))
        end
        ns.Print(format("%d emotes in total.", ns.count))
    else
        ns.Print("Unknown command. Type /7tv for help.")
    end
end

SLASH_SEVENTVEMOTES1 = "/7tv"
SLASH_SEVENTVEMOTES2 = "/seventv"
SlashCmdList.SEVENTVEMOTES = Slash
