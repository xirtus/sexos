/* helpers.h for SexOS: replaces vendor/ladspa-t5/helpers.h.

   The original opened /dev/shm/t5_* with open/ftruncate/mmap so an outside
   process could change parameters while the plugin runs. A SexOS plugin has
   no files; Moan supplies parameters through the control ports. The gain
   helper and descriptor cleanup are kept as upstream wrote them. */
#include <math.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <ladspa.h>

typedef struct {
    long s;
    long ns;
    LADSPA_Data * mmap;
} TimeMmapStruct;

static float dbToGainFactor(float db) {
  return pow(10, db/20.0);
}

static void cleanupMmapFile(char pluginname[], float mmapfname, long s, long ns) {
    (void)pluginname; (void)mmapfname; (void)s; (void)ns;
}

static TimeMmapStruct setupMmapFile(char pluginname[], float mmapfname, int portcount) {
    TimeMmapStruct ret = { 0, 0, NULL };
    (void)pluginname; (void)mmapfname; (void)portcount;
    return ret;
}

static void deleteDescriptor(LADSPA_Descriptor* descriptor) {
    unsigned long index;
    if (descriptor) {
        free((char *)descriptor->Label);
        free((char *)descriptor->Name);
        free((char *)descriptor->Maker);
        free((char *)descriptor->Copyright);
        free((LADSPA_PortDescriptor*)descriptor->PortDescriptors);
        for (index = 0; index < descriptor->PortCount; index++)
            free((char *)(descriptor->PortNames[index]));
        free((char **)descriptor->PortNames);
        free((LADSPA_PortRangeHint*)descriptor->PortRangeHints);
        free(descriptor);
    }
}
