# Complete catalog parsing for ordinary program gates

After the Files binary fix, foundation c6d70112 built and booted successfully. Several program gates then rejected actual successor16 because program_role_facts parsed literal Incarnation rows but omitted the catalog's quil(...) constructor rows. The parser now includes that fixed constructor and checks the declared catalog count, refusing unknown/unparsed constructors. All sealed BOOT fields, exact task/surface/source-pin and uniqueness checks remain enforced.

Seven targeted parser cases pass, including a constructor row, missing BOOT record and unknown constructor negative. Replaying the original failed CI std-serial.log now yields all eight reusable program rows; this is parser replay, not a rerun or native gate pass. Original build/runtime evidence retained under desktop-recorder/foundation-cic6d. CI host custody gate now includes these parser controls.
