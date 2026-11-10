# Percentage dimensions for replaced HTML elements

A native responsive-frame probe exposed a distinct layout issue: an absolute
iframe with width:50% and height:120px got a zero-width box and was never loaded.
The failed guest trace is retained under sexos-evidence/web-live-viewport/
frames-responsive. This is separate from the ordinary Web program's fixed-size
viewport, which is addressed by PR #152.

Atomic replaced-element preparation used resolve(0), confusing its temporary
origin with the containing block's dimensions. Pass the real containing width
through block and inline preparation, including nested inline spans. Resolve
percentage heights only when the containing height is definite; otherwise keep
intrinsic aspect/default sizing. Percentage heights remain indefinite during
intrinsic contribution measurement instead of making natural image width zero.
Fixed sizes and intentional zero percentages keep their existing semantics.

The basis is CSS2 sections 10.2 and 10.5 (containing width; definite containing
height) and replaced sizing in 10.3.2/10.6.2:
https://www.w3.org/TR/CSS2/visudet.html

All 35 existing engine tests plus seven new replaced-percentage tests pass. New
cases cover viewport/containing width, nested inline image aspect ratio, definite
and indefinite percentage height, intrinsic shrink-to-fit, default/fixed sizes
and intentional zero width. The first source-check attempt had a signature-edit
compile error; it was corrected and both logs are retained. No native success or
third-party embed readiness is claimed yet. Native qualification and final
master/viewport integration remain pending; no compiler/VM is running for this
branch while the JIT privacy agent owns the native window.

No kernel, IPC, sandbox, request origin, website source or header changes. Initial
base is merged peer-state master d92091cb. This is an independent layout topic.
