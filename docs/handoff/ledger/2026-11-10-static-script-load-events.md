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
Python syntax and diff checks pass; native guest validation is pending an
exclusive QEMU window. No kernel/ABI or resource access-policy changes.

Reference: [HTML execute the script element](https://html.spec.whatwg.org/multipage/scripting.html#execute-the-script-element),
which restores currentScript and fires load for an external resource.
This fixes a general lifecycle gap. Its relationship to the real YouTube
`writeEmbed` bootstrap failure remains a hypothesis until replayed; it is not a
player/playback claim.
