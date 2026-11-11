# Checked Files epoch

Authority-sensitive Files runtime now uses checked self_epoch rather than the presentation helper that maps syscall failure to zero restarts. Query errors, zero and out-of-range epochs refuse authority; the checked SDK helper rejects increment overflow. A failed query on an all-zero KEEP page cannot initialize a cold Free journal. New actual runtime fixture covers failure -> quarantine -> no image I/O. Current-head CI is the validation; local compilers deliberately held while matched UX timing runs.
