# Recording dependencies in extracted Display fixtures

The recorder CI actual pipeline and Scene lease fixtures extracted Main calls
to RECORDING_CAPTURE without including its controller or marker dependencies.
Both now extract Main's actual controller static and import the actual recording
controller, marker, protocol and bank modules. Input receipts hash those modules.

Recording stays unattached in these non-recording fixtures. Newly required held
peer/task, region and event boundaries panic if reached; they grant no authority
and cannot silently accept active capture. Existing lifecycle cleanup boundaries
and all production-path assertions/mutation controls remain unchanged. Active
recording custody remains covered by its dedicated actual-controller fixture.

Validation: embedded gate Python syntax and git diff --check pass. Rust gates
and CI are pending; no compiler or VM was run while JIT owns the heavy slot.
No production code or behavior changed.
