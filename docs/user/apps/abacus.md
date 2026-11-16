# Abacus (Calculator)

**Abacus** (`apps/abacus`, `/bin/abacus`) is the SexOS calculator, built on the Weft
toolkit. Type a sum or click the keypad, reuse past answers, and keep a number in
memory. The old `sex-calc` name still starts it.

## Launching Abacus

- **From Spindle**: `run abacus` (or `run sex-calc`). Spindle's own `calc <expr>`
  command works sums out in the terminal.
- **From Voyeur**: press **Super+D** and type `abacus`.
- **In the panel**: `run abacus tray` also puts an **=** item in the panel. Its menu
  shows the last answer, clears, goes back to Scientific, or asks for a sum: type one
  into the notification that comes up and press **Enter** to have it worked out.

## Modes

The buttons under the menu bar (or the **Mode** menu) switch between:

- **Scientific**: the sums described below.
- **Programmer**: exact whole numbers in 8, 16, 32 or 64-bit words (the **Words**
  menu), `0x`/`0b`/`0o` literals, `& | ^ ~ << >>` (here `^` is XOR), and output in
  decimal, hex, binary or octal.
- **Convert**: units, such as `32 F to C` or `2.5 km to m` (length, mass, time,
  temperature and storage).

A past answer picked from the history comes back in the mode it was worked out in.

## Typing sums

Type into the expression line and press **Enter**. Abacus understands:

- `+ - * / ^`, and `× ÷ −` as the keypad shows them; brackets; a leading minus.
- `%` as percent (`200*15%` is 30) and `mod` for the remainder (`7 mod 3` is 1).
- `!` for factorials (`5!`), and sums written without `*` (`2pi`, `3(4+1)`).
- Constants: `pi` (or `π`), `e`, `ans` (the last answer) and `mem` (the memory).
- Functions: `sqrt` (or `√`), `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `ln`,
  `log`, `log2`, `exp`, `exp2`, `sinh`, `cosh`, `tanh`, `cbrt`, `abs`, `round`,
  `floor`, `ceil`, `trunc`. `sin 30` works without brackets.
- Angles in degrees (the default) or radians. In degrees, `sin(180)` is exactly 0
  and `tan(90)` is refused.

Straight after an answer, a sum that starts with an operator carries on from it:
`+8` means `ans+8`. A sum that starts with a number starts afresh.

When something is wrong, the result line says what: *Brackets don't match*, *Can't
divide by zero*, *Out of range*, *Too big*, *Unknown: name* or *Check the sum*.

## The keypad

| Row | Keys |
| --- | --- |
| Memory | **MC** clear, **MR** recall, **M+** add, **M−** subtract, **±** change sign |
| Editing | **C** clear, **⌫** backspace, `(` `)` `%` |
| Functions | `sin` `cos` `tan` `√` `^` |
| Numbers | `0`–`9`, `.`, `÷ × − +`, `π`, `ln`, `log`, `ans`, **=** |

M+ and M− use what the expression line works out to, or the last answer when the line
is empty. The line above the result shows the memory while it holds a value.

## Menus

- **Edit**: Copy Result (**Ctrl+Shift+C**), Paste, Clear (**Esc**), Clear History.
- **Functions**: Square (x²), Reciprocal (1/x), Change Sign, and functions and
  constants to put in the expression line (asin, acos, atan, exp, abs, n!, mod, e, mem).
- **Memory**: Memory Clear, Recall, Add and Subtract.
- **View**: Degrees, Radians, Switch Degrees and Radians (**Ctrl+D**), and the command
  palette (**Ctrl+Shift+P**), which finds every command by name.

## History

Every answer goes into the History list beside the keypad. **Up** and **Down** pick a
past answer and **Enter** puts it into the expression line; so does a double-click.
Negative answers go in with brackets, so they stay one value.

## Keys

| Key | Does |
| --- | --- |
| **Enter** | Work the sum out (or put in the picked answer) |
| **Esc** | Clear |
| **Up / Down** | Pick a past answer |
| **Ctrl+Up** | Put the last answer in |
| **Ctrl+D** | Switch degrees and radians |
| **Ctrl+Shift+C** | Copy the result |
| **Ctrl+Shift+P** | Command palette |
| **F11** | Close (Shell) |
