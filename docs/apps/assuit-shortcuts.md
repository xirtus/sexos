# Assuit keyboard shortcuts

Press **Ctrl+Shift+/** in Assuit for the same reference in the document window.
Press it again, or Esc, to return to editing.

| Shortcut | Action |
| --- | --- |
| Ctrl+S | Save the document |
| Ctrl+Shift+Q | After a failed close-time recovery write, explicitly discard unsaved changes and close |
| Ctrl+Shift+E | Save As; the extension selects DOCX, ODT, RTF, HTML, Markdown, text or PDF |
| Ctrl+P | Export the pages to PDF beside the document |
| Ctrl+Shift+P | Enter a page range such as `1-3,5` and print it to a separate PDF |
| Ctrl+Alt+P | Open or close print preview; Left/Right or PageUp/PageDown move between pages, Ctrl+P exports PDF, Esc returns to editing |
| Ctrl+Shift+L | Set the paper to Letter, Legal or A4, in portrait or landscape orientation |
| Ctrl+Shift+M | Set normal, narrow, moderate, wide or mirrored margins, or enter top/bottom/left/right inches |
| Ctrl+Alt+N | Add dynamic page numbers at `top` or `bottom`, aligned `left`, `center`, or `right`; append `of` for Page X of Y; `remove top/bottom` removes generated-only content in a single-section document; `start N` (1–32767) restarts numbering, `continue` resumes from the previous section, `format decimal/lower roman/upper roman/lower letter/upper letter` changes the current section's number style; `first on/off` uses a separate first-page header/footer in this section, and `even on/off` uses separate even-page headers/footers throughout the document |
| Ctrl+Alt+H / Ctrl+Alt+F | Edit the header or footer shown on the current page. Type and format text around existing fields; Esc returns to the body. An empty newly opened region is removed on exit. |
| Ctrl+Alt+S | Set the current or selected paragraphs' line multiple (0.5–3), or an `exact` or `minimum` line height (6–72 pt), then space before and after (0–72 pt): `1.5 0 8`, `exact 18 0 8`, or `minimum 18 0 8` |
| Ctrl+Alt+I | Set left, right, and relative first-line indents in inches. Use `0.5 0 -0.25` for a hanging indent. |
| Ctrl+Shift+N | Insert a linked footnote at the body caret. Enter its text, continue editing the note on the page, then press Esc to return to the body. |
| Ctrl+Alt+E | Insert a linked endnote at the body caret. Enter its text, continue editing, then press Esc to return to the body. |
| Ctrl+Alt+Shift+E | Edit a linked footnote or endnote from its body reference, including after reopening a document. Esc returns to the body. |
| Ctrl+Alt+Shift+N | Convert all footnotes to endnotes, all endnotes to footnotes, or swap both kinds. Enter `endnote`, `footnote`, or `swap`. |
| Ctrl+Alt+Shift+F | Set note placement or numbering for the document. Examples: `footnote format roman`, `footnote restart page`, `endnote start 3`, `endnote position section`. |
| Ctrl+K | Link selected text or insert a new link. At a caret inside an existing link, selects its full label and prefills its address for editing. Enter an `https://`, `http://`, or `mailto:` address, optionally followed by display text after a space; Ctrl+A clears the prompt. |
| Ctrl+Alt+K | Remove the link from selected text while keeping the words. |
| Ctrl+Alt+T | Insert a generated table of contents from heading styles with `insert 1` through `insert 9` (maximum heading level), `update` after edits, or `remove` it. |
| Ctrl+Alt+L | Edit the current list: `bullet`, `numbered`, `numbered paren`, `upper letter`, `lower letter`, `lower roman`, `legal`, `off`, `restart`, or `level 1` through `level 9` (1 is the outermost level) |
| Ctrl+Alt+Right / Ctrl+Alt+Left | Move the list item at the caret one level deeper or shallower (levels 1–9) |
| Ctrl+T | Insert a table by entering rows × columns |
| Ctrl+Shift+T | Edit the table at the caret: select/insert/delete rows or columns; `merge cells` for a selected range; `split 2` through `split 10` for the current cell; `header on` or `header off` to repeat a leading row across pages; `shade RRGGBB` for cell fill; `border all`, `outside`, `inside`, or `none` for DOCX table borders; enter `style` to browse the native DOCX table-style gallery, or `style NAME` to apply one directly |
| Ctrl+Shift+I | Insert an image from Assuit's granted folder |
| Ctrl+Shift+R | Describe the selected picture with alternative text (1–500 characters) |
| Ctrl+Shift+O | Set the selected picture's width in inches, preserving proportions |
| Ctrl+Shift+C | Crop the selected picture by left, top, right, bottom percentages; `0 0 0 0` resets |
| Ctrl+Shift+W | Set selected picture text flow: inline, square, or top-and-bottom |
| Ctrl+Shift+S | Choose a paragraph style |
| Ctrl+B, Ctrl+I, Ctrl+U | Bold, italic, underline |
| Ctrl+Z, Ctrl+Y | Undo, redo |
| Ctrl+A | Select all |
| Ctrl+C, Ctrl+X, Ctrl+V | Copy, cut, and paste text through SexOS's shared clipboard; a same-document paste keeps formatting when the clipboard still matches |
| Ctrl+F, Ctrl+H | Find, find and replace |
| Ctrl+G | Go to page |
| Ctrl+Shift+Y | Check spelling and choose a suggestion |
| Ctrl+Shift+U | Show or hide continuous spelling marks on the editable page; Assuit remembers the choice across launches |
| A (during a spelling suggestion) | Add the selected word to Assuit's persistent dictionary |
| Ctrl+Shift+K | Set the document default and selected text's proofing language to `en-US`, `en-GB`, `en-AU`, `en-CA`, an installed language such as `de-DE`, or `none` for DOCX |
| Ctrl+Shift+D | Manage Assuit's custom dictionary: A adds a word, Up/Down chooses, Delete removes, Esc closes |
| Ctrl+=, Ctrl+-, Ctrl+0 | Zoom in, out, or reset |

The status bar shows the most common actions. Selection, cursor movement and
editing commands also follow the WordCraft document engine's keymap.

For another language, place a licensed Hunspell pair such as `de-DE.aff` and
`de-DE.dic` in Assuit's `assuit-dictionaries` folder using Loom. Reopen the
language prompt and enter the matching language-region tag. Assuit checks that
WordCraft can load the dictionary before changing the document.
