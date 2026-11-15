// SPDX-License-Identifier: GPL-3.0-only
//! The built-in commands: names, what they do, their flags and argument shapes.
//! The parser, `help`, completion and highlighting all read this table.

/// How a command's arguments parse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// Separate values: words, strings, numbers, lists, `( )`.
    Atoms,
    /// The rest is one row condition with operators: `where size > 1mb and ...`.
    Condition,
    /// A column name, then one expression: `update size (size * 2)`.
    ColumnThenExpr,
}

/// What a command's positional arguments are, for completion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgKind {
    None,
    Path,
    Dir,
    Column,
    Command,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Flag {
    pub long: &'static str,
    pub short: Option<char>,
    /// Takes a value: `--separator ';'`.
    pub value: bool,
    pub desc: &'static str,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Signature {
    pub name: &'static str,
    /// Argument synopsis for `help`.
    pub args: &'static str,
    pub desc: &'static str,
    pub category: &'static str,
    pub flags: &'static [Flag],
    pub shape: Shape,
    pub completes: ArgKind,
    /// Reads only: safe to run while completing a later stage's columns.
    pub pure: bool,
}

impl Signature {
    pub fn flag(&self, long: &str) -> Option<&'static Flag> {
        self.flags.iter().find(|f| f.long == long)
    }
    pub fn short_flag(&self, short: char) -> Option<&'static Flag> {
        self.flags.iter().find(|f| f.short == Some(short))
    }
}

const fn sig(name: &'static str, args: &'static str, desc: &'static str, category: &'static str, flags: &'static [Flag], shape: Shape, completes: ArgKind, pure: bool) -> Signature {
    Signature { name, args, desc, category, flags, shape, completes, pure }
}

const fn f(long: &'static str, short: Option<char>, value: bool, desc: &'static str) -> Flag {
    Flag { long, short, value, desc }
}

use ArgKind as K;
use Shape as S;

pub static COMMANDS: &[Signature] = &[
    // Files and folders (through the host).
    sig("ls", "[path or pattern]", "List a folder as a table: name, type, size, modified", "files", &[f("all", Some('a'), false, "include hidden entries (names starting with .)")], S::Atoms, K::Path, true),
    sig("cd", "[folder]", "Change the current folder", "files", &[], S::Atoms, K::Dir, false),
    sig("pwd", "", "The current folder", "files", &[], S::Atoms, K::None, true),
    sig("mkdir", "<folder>...", "Make folders", "files", &[], S::Atoms, K::Path, false),
    sig("rm", "<path>...", "Remove files and folders", "files", &[f("recursive", Some('r'), false, "remove a folder and what it holds")], S::Atoms, K::Path, false),
    sig("mv", "<from> <to>", "Move or rename a file or folder", "files", &[], S::Atoms, K::Path, false),
    sig("cp", "<from> <to>", "Copy a file (a folder with -r)", "files", &[f("recursive", Some('r'), false, "copy a folder and what it holds")], S::Atoms, K::Path, false),
    sig("touch", "<file>...", "Make empty files that do not exist yet", "files", &[], S::Atoms, K::Path, false),
    sig("save", "<file>", "Write the input to a file (.json, .csv, .tsv convert tables)", "files", &[f("append", Some('a'), false, "add to the end of the file"), f("force", Some('f'), false, "replace a file that exists"), f("raw", Some('r'), false, "write text as is, without converting by extension")], S::Atoms, K::Path, false),
    sig("open", "<file>", "Read a file; .json .toml .csv .tsv become structured, .txt becomes lines", "files", &[f("raw", Some('r'), false, "keep the contents as text or bytes")], S::Atoms, K::Path, true),
    // System.
    sig("sys", "", "Information about this machine", "system", &[], S::Atoms, K::None, true),
    sig("ps", "", "Running programs", "system", &[], S::Atoms, K::None, true),
    sig("jobs", "", "Background jobs started with `&`", "system", &[], S::Atoms, K::None, true),
    sig("fg", "[job]", "Bring a background job to the front and wait for it", "system", &[], S::Atoms, K::None, false),
    sig("kill", "<job>", "Stop a background job", "system", &[], S::Atoms, K::None, false),
    sig("date now", "", "The current date and time", "system", &[], S::Atoms, K::None, true),
    // Filters.
    sig("where", "<condition>", "Keep the rows where the condition holds: where size > 1mb and name =~ rs", "filters", &[], S::Condition, K::Column, true),
    sig("select", "<column>...", "Keep only these columns", "filters", &[], S::Atoms, K::Column, true),
    sig("reject", "<column>...", "Drop these columns", "filters", &[], S::Atoms, K::Column, true),
    sig("get", "<cell path>", "Take a column, row or field: get name, get 0.name", "filters", &[], S::Atoms, K::Column, true),
    sig("sort-by", "<column>...", "Sort rows by columns (or a list by its values)", "filters", &[f("reverse", Some('r'), false, "largest first"), f("ignore-case", Some('i'), false, "sort text without regard to case")], S::Atoms, K::Column, true),
    sig("first", "[n]", "The first row, or the first n rows", "filters", &[], S::Atoms, K::None, true),
    sig("last", "[n]", "The last row, or the last n rows", "filters", &[], S::Atoms, K::None, true),
    sig("skip", "<n>", "Leave out the first n rows", "filters", &[], S::Atoms, K::None, true),
    sig("take", "<n>", "Keep the first n rows", "filters", &[], S::Atoms, K::None, true),
    sig("length", "", "How many rows or items", "filters", &[], S::Atoms, K::None, true),
    sig("reverse", "", "Reverse the order of rows", "filters", &[], S::Atoms, K::None, true),
    sig("uniq", "", "Drop repeated rows", "filters", &[], S::Atoms, K::None, true),
    sig("group-by", "<column>", "Group rows into a record by a column's value", "filters", &[], S::Atoms, K::Column, true),
    sig("update", "<column> <expression>", "Set a column in every row: update size (size * 2)", "filters", &[], S::ColumnThenExpr, K::Column, true),
    sig("insert", "<column> <expression>", "Add a column to every row: insert big (size > 1mb)", "filters", &[], S::ColumnThenExpr, K::Column, true),
    sig("wrap", "<column>", "Make a list into a table with one column", "filters", &[], S::Atoms, K::None, true),
    sig("columns", "", "The column names of a table or record", "filters", &[], S::Atoms, K::None, true),
    sig("describe", "", "The type of the input", "filters", &[], S::Atoms, K::None, true),
    // Math.
    sig("math sum", "", "Add up numbers, sizes or durations (each column of a table)", "math", &[], S::Atoms, K::None, true),
    sig("math avg", "", "The average", "math", &[], S::Atoms, K::None, true),
    sig("math min", "", "The smallest", "math", &[], S::Atoms, K::None, true),
    sig("math max", "", "The largest", "math", &[], S::Atoms, K::None, true),
    // Text.
    sig("str contains", "<text> [column]...", "Whether the text holds a piece", "strings", &[f("ignore-case", Some('i'), false, "without regard to case")], S::Atoms, K::Column, true),
    sig("str replace", "<find> <replace> [column]...", "Replace the first match (every one with -a)", "strings", &[f("all", Some('a'), false, "replace every match")], S::Atoms, K::Column, true),
    sig("str upcase", "[column]...", "Text to upper case", "strings", &[], S::Atoms, K::Column, true),
    sig("str downcase", "[column]...", "Text to lower case", "strings", &[], S::Atoms, K::Column, true),
    sig("str trim", "[column]...", "Remove space at both ends", "strings", &[], S::Atoms, K::Column, true),
    sig("str length", "[column]...", "Characters in the text", "strings", &[], S::Atoms, K::Column, true),
    sig("split row", "<separator>", "Split text into a list at a separator", "strings", &[], S::Atoms, K::None, true),
    sig("lines", "", "Split text into a list of lines", "strings", &[], S::Atoms, K::None, true),
    // Formats.
    sig("to json", "", "Structured data as JSON text", "formats", &[f("raw", Some('r'), false, "on one line, without indentation")], S::Atoms, K::None, true),
    sig("to csv", "", "A table as comma-separated text", "formats", &[], S::Atoms, K::None, true),
    sig("to tsv", "", "A table as tab-separated text", "formats", &[], S::Atoms, K::None, true),
    sig("to text", "", "Anything as plain text", "formats", &[], S::Atoms, K::None, true),
    sig("from json", "", "Parse JSON text", "formats", &[], S::Atoms, K::None, true),
    sig("from csv", "", "Parse comma-separated text with a header row", "formats", &[f("separator", Some('s'), true, "the column separator, one character"), f("noheaders", Some('n'), false, "the first row is data; columns are named column0, column1, ...")], S::Atoms, K::None, true),
    sig("from tsv", "", "Parse tab-separated text with a header row", "formats", &[f("noheaders", Some('n'), false, "the first row is data")], S::Atoms, K::None, true),
    sig("from toml", "", "Parse TOML text", "formats", &[], S::Atoms, K::None, true),
    sig("table", "", "Draw the input as a table, as text", "formats", &[], S::Atoms, K::None, true),
    // Values.
    sig("echo", "<value>...", "The values given (a list when more than one)", "core", &[], S::Atoms, K::None, true),
    sig("seq", "<first> [step] <last>", "A list of numbers from first to last", "core", &[], S::Atoms, K::None, true),
    sig("help", "[command]", "List the commands, or explain one", "core", &[], S::Atoms, K::Command, true),
];

/// Words that only begin two-word commands (`math sum`).
pub const NAMESPACES: &[&str] = &["math", "str", "split", "to", "from", "date"];

/// Words with meaning of their own in conditions and statements.
pub const KEYWORDS: &[&str] = &["let", "and", "or", "xor", "not", "in", "not-in", "mod", "starts-with", "ends-with", "true", "false", "null"];

pub fn lookup(name: &str) -> Option<&'static Signature> {
    COMMANDS.iter().find(|s| s.name == name)
}

/// The subcommands under a namespace word: `math` gives `math sum`, `math avg`, ...
pub fn subcommands(namespace: &str) -> impl Iterator<Item = &'static Signature> + '_ {
    COMMANDS.iter().filter(move |s| s.name.split_once(' ').map(|(ns, _)| ns == namespace).unwrap_or(false))
}
