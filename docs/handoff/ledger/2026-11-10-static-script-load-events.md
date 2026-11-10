# Initial external classic-script load events

The initial page-script runner restored `document.currentScript` but never fired
an external script element's load event. Dynamically inserted resources already
had a deferred completion event. Initial external classic scripts now use a synchronous
`scriptload` dispatch after restoring currentScript and before the next script.
A JavaScript exception still permits load, because the resource loaded; terminal
VM traps do not dispatch another callback. Inline scripts do not receive load.

The deterministic classic-resource gate adds an initial external script which
registers its load handler and throws. Its following inline script must observe
the completed handler, that handler must see currentScript null, and an inline
load handler must never run. The existing security/error/navigation cases remain.
Python syntax/diff checks, native FFmpeg build and executable key-byte scanning
pass. The extended native four-core guest gate passes all lifecycle and existing
security/error/navigation cases. Source `2da1b041`, Web SHA256
`a950b8408ef7edc3508d5e6f100188c1e603700e6b749fe00960aa3388f172ce`;
separate boot source 55ae0526 ISO SHA256
`a47d5294d1810f3626d5a45c1cd2a95a0835a1be0c75e0bb0a433893f2daea16`,
ABI-input manifest `cd551118355cb882...` matches. Receipts are in
`Projects/sexos-evidence/pr143-classic-sync-native-gate`. No kernel/ABI or resource access-policy changes.

Reference: [HTML execute the script element](https://html.spec.whatwg.org/multipage/scripting.html#execute-the-script-element),
which restores currentScript and fires load for an external resource.
This fixes a general lifecycle gap. Its relationship to the real YouTube
`writeEmbed` bootstrap failure remains a hypothesis until replayed; it is not a
player/playback claim.
