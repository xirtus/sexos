# Calendar gate uses the Date run that session restore reopens

After the gate cuts power with Date still open, Spindle's session restore
starts Date again on the next boot. The gate then ran `date` a second time
and waited for a launch line belonging to the restored run, which had been
logged before it started looking. The gate now waits for the restored run
(`[spindle.session.restore]` and `[spindle.session.started]`) and checks the
power-cut event in that window. Gate-only change.
