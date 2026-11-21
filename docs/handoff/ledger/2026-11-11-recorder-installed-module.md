# Recorder module in the installed boot image

The opt-in ISO declares `/programs/record` as a BOOT module, but the boot-partition
builder previously copied only servers, apps and Wi-Fi firmware. Installation
would retain the module reference without its bytes.

The builder now copies exactly the configured recorder module into `/programs`,
rejects missing, duplicate or unsupported program module references before image
creation, and uses the existing 32 MiB boot-slot capacity for this profile.
Unreferenced programs are not imported. FAT copy errors still stop the build;
the partition is never silently enlarged.

Two targeted Python controls passed in 0.430 seconds. The actual mtools/FAT
fixture checks normal and recorder profiles, exact copied bytes, retained module
reference, removed installer-image reference, and absence of unrelated programs.
The fixture contains labelled synthetic payloads, not an executable recorder.
It does not prove the full recorder plus firmware fits, boots from an installed
disk, or records media. Those remain native profile/install gates. The controls
are included in the existing recorder host gate.
