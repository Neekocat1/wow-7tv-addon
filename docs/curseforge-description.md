# 7TV Emotes

**Twitch-style 7TV emotes in World of Warcraft: Forever chat.** Type an emote's name in chat and everyone with the addon sees the emote instead of the word, animated emotes included.

## Features

- **Emotes in chat:** emote names in chat messages are replaced with the image, at a size you choose (12 to 64 px). Animated emotes play in chat.
- **Autocomplete:** type `:` and two letters of an emote name in the chat box. **Tab** cycles through matches, another `:` accepts.
- **Hover and reuse:** hover an emote in chat to see its name; shift-click it to put it in your chat box.
- **The /7tv window:** browse and search all your emotes with animated previews, see where each one came from, hide ones you don't want, and manage your emote sources.
- **Minimap button:** left-click opens the window, right-click opens settings, drag it around the minimap. Also listed in the minimap's addon menu.
- **Lightweight:** optional compressed textures use about a quarter of the memory.

## Getting emotes

WoW addons can't download anything, so this addon **doesn't include any emotes by itself**. There are two ways to get them (links at the bottom of this page):

**Option 1: emote pack, nothing to install.** On the pack builder web page, enter one or more Twitch channels (and/or individual emotes), click Build, and download a zip. Extract it into `World of Warcraft\_forever_\Interface\AddOns` and restart WoW. To change your emotes, build a new pack.

**Option 2: the 7TV Emotes Helper app (Windows).** A small free, open-source app. Run it once: it finds WoW: Forever, sets up the emote folder and runs quietly in the system tray. After that you add emotes from inside the game:

| Command | What it does |
|---|---|
| `/7tv add <7tv.app link or name> [new name]` | add one emote, optionally under another name |
| `/7tv channel <twitch name>` | add a streamer's 7TV emotes |
| `/7tv unchannel <twitch name>` | remove them again |
| `/7tv remove <name>` | remove an emote |

7TV's global emotes are added automatically. Requests are sent when you reload; a status bar above chat shows the helper's progress and tells you when to reload again.

## Other commands

| Command | What it does |
|---|---|
| `/7tv` | open the 7TV window |
| `/7tv search <text>` | list matching emotes in chat |
| `/7tv size <12-64>` | emote size in chat |
| `/7tv on` / `off` | turn emotes in chat on or off |
| `/7tv anim`, `/7tv complete`, `/7tv tips` | toggle animation, autocomplete, hover tooltips |
| `/7tv disable <name>` / `enable <name>` | hide or show one emote |
| `/7tv minimap` | show or hide the minimap button |
| `/7tv help` | all commands |

## Good to know

- Only players with the addon see the images, and only for emotes they have too; everyone else sees the word.
- Chat in dungeons and raids stays as text: the game doesn't let addons change it there.
- Only 7TV emotes are supported (not BetterTTV or FrankerFaceZ). Many of those exist on 7TV too: add them by name with `/7tv add <name>`.
- Zero-width (overlay) emotes are skipped.
- After installing an emote pack, or after the helper adds emotes for the first time, restart WoW once so the game can see the new image files.

This addon isn't affiliated with 7TV, Twitch or Blizzard. Emotes belong to their creators on 7TV.

---

**Emote pack builder:** https://neekocat1.github.io/wow-7tv-addon/
**Source code and the 7TV Emotes Helper app:** https://github.com/Neekocat1/wow-7tv-addon
