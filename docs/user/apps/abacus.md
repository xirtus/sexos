# Abacus (Calculator)

> **Identity Note**: **Abacus** (`apps/sex-calc`, `/bin/abacus`, function key **F9**) is the native SexOS mathematical calculation suite, named under Andreas's classical computing instruments theme.

**Abacus** provides exact rational arithmetic, scientific computation, and programmer base conversions. Designed for precision, Abacus avoids floating-point rounding errors when calculating exact fractions, ratios, and currencies.

---

## Launching Abacus

- **Global Shortcut**: Press **`F9`** to summon the floating Abacus window.
- **From Spindle**: Run `run abacus` or use the built-in `calc <expr>` command.
- **From Voyeur**: Press **`Super+D`** and type math equations directly (e.g., `42 * 1337`).

---

## Operating Modes

Press **`Tab`** in Abacus to switch between operational modes:

### 1. Standard & Exact Arithmetic
- Standard operations: `+`, `-`, `*`, `/`, `%` (modulo), `**` (exponentiation).
- **Exact Rational Results**: Computes fractions and currency exactly without binary floating-point drift (e.g., `0.1 + 0.2 = 0.3`).
- **Parentheses**: Full expression nesting: `((12 + 8) * 3) / 2`.

### 2. Scientific Math
- **Trigonometric Functions**: `sin(x)`, `cos(x)`, `tan(x)`, `asin(x)`, `acos(x)`, `atan(x)`.
- **Logarithms & Powers**: `ln(x)`, `log10(x)`, `log2(x)`, `exp(x)`, `sqrt(x)`.
- **Constants**: Built-in definitions for `pi` (3.14159...) and `e` (2.71828...).

### 3. Programmer & Radix Conversion
- **Base Conversions**: Convert numbers dynamically between Decimal, Hexadecimal (`hex`), Binary (`bin`), and Octal (`oct`).
  - Example: `255 in hex` → `0xFF`
  - Example: `0b101010 in dec` → `42`
- **Bitwise Operations**: `AND` (`&`), `OR` (`|`), `XOR` (`^`), `NOT` (`~`), bit shifts (`<<`, `>>`).

---

## Keyboard Controls

| Key | Action | Description |
|---|---|---|
| **`Enter`** | **Calculate** | Evaluate the entered expression and append result to history tape. |
| **`Up / Down`** | **History Recall** | Cycle through previous equations and recalled answers (`ans`). |
| **`Esc`** | **Clear** | Clear current input line. |
| **`Tab`** | **Switch Mode** | Cycle between Standard, Scientific, and Programmer modes. |
| **`Ctrl+C`** | **Copy Result** | Copy evaluated answer to the system clipboard. |
| **`Q`** | **Quit** | Close Abacus. |
