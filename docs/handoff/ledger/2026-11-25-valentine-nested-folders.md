# Valentine: browse nested local folders

- Ctrl+G now cycles the top level and nested local mail folders inside Valentine's existing Files grant, bounded to four levels and 64 folders. Folder components reject separators, dot traversal, control bytes, and overlong names. Drafts save in the selected nested folder.
- The focused `sex-mail` host test covers valid nested paths and traversal/depth refusal. GitHub CI's Valentine QEMU gate visits a nested folder, captures `valentine-nested-folder.ppm`, and verifies a draft saved there.
- This changes only local mail. Live IMAP/SMTP still needs a boot-routed Multipass service, verified TLS dialer, account/grant UI, Valentine client, and Bell mail events.
