# Spindle gap list

Compared with a daily terminal on Linux or macOS, Spindle still needs broader
command editing, accessible session navigation, and safer treatment of private
program arguments across diagnostics and restore state. This change closes the
last gap for recognized `--private` Kaleidoscope launches: input receipts carry
structure only, and neither command history nor session restore retains the URL.
Shell Yarn's integrated prompt receives the same input redaction and skips a
private launch-shaped command in its in-memory history and output.
