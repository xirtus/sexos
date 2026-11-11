# Recorder module across installed boot entries

Actual opt-in build at6b0200a5 failed before FAT creation because the packaging parser treated the default and compact Limine entries' references to the same recorder as a duplicate. The parser now requires uniqueness within each entry and copies the exact common module once. Unknown paths, missing files and duplicate references within an entry still fail.

Three targeted controls pass, including the repository's actual default+compact config and an actual mtools FAT round trip with both entries. These are labelled packaging fixtures, not native OS qualification. Failure JSON and raw log remain in desktop-recorder/native-6b020; no guest ran from that candidate.
