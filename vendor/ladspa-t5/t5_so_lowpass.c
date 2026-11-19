/* t5_so_lowpass.c

   Free software by Juergen Herrmann, t-5@t-5.eu. Do with it, whatever you
   want. No warranty. None, whatsoever. Also see license.txt .

   This LADSPA plugin provides a Second Order low pass filter.

   This file has poor memory protection. Failures during malloc() will
   not recover nicely. */

/*****************************************************************************/

#include <math.h>
#include <stdlib.h>
#include <stdio.h>
#include <unistd.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/types.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <time.h>
#include <ladspa.h>
#include "helpers.h"
#include "secondorder.h"


static void plugin_init(void) __attribute__((constructor)); 
static void plugin_deinit(void) __attribute__((destructor)); 


/* setup shared memory area to enable parametrization at runtime from external processes */
void setupMmapFileForSoLowpass(SoLowHighPass* instance) {
    TimeMmapStruct ret;
    ret = setupMmapFile("SoLowpass", *(instance->mmapFname), PORTCOUNT); 
    instance->mmapArea = ret.mmap;
    instance->created_s = ret.s;
    instance->created_ns = ret.ns;
}


/* Run the filter algorithm for a block of SampleCount samples. */
void runSoLowpass(LADSPA_Handle ladspaHandle, unsigned long sampleCount) {
    BiquadCoeffs coeffs;
    SoLowHighPass* instance;
    instance = (SoLowHighPass*)ladspaHandle;
    if (instance->mmapArea == NULL && *(instance->mmapFname) != 0.0) {
        setupMmapFileForSoLowpass(instance);
    }
    // Butterworth characteristic, Q = 0.707...
    coeffs = calcCoeffs2ndOrderLowpass(*(instance->f), 0.7071067811865476, instance->sampleRate);
    runSoBiquad(instance, sampleCount, coeffs);
}


/* Throw away a SoLowPass instance. */
void cleanupSoLowpass(LADSPA_Handle ladspaHandle) {
    SoLowHighPass* instance;
    instance = (SoLowHighPass*)ladspaHandle;
    if (instance->mmapArea != NULL) {
        cleanupMmapFile("SoLowpass",
                        *(instance->mmapFname),
                        instance->created_s,
                        instance->created_ns);
        free(ladspaHandle);
    }
}


LADSPA_Descriptor * soLowpassInstanceDescriptor = NULL;


/* called automatically when the plugin library is Second loaded. */
void plugin_init() {

    char** portNames;
    LADSPA_PortDescriptor* portDescriptors;
    LADSPA_PortRangeHint* portRangeHints;
    
    soLowpassInstanceDescriptor
      = (LADSPA_Descriptor *)malloc(sizeof(LADSPA_Descriptor));

    if (soLowpassInstanceDescriptor != NULL) {
    
        soLowpassInstanceDescriptor->UniqueID
            = 5550;
        soLowpassInstanceDescriptor->Label
            = strdup("so_lowpass");
        soLowpassInstanceDescriptor->Properties
            = LADSPA_PROPERTY_HARD_RT_CAPABLE;
        soLowpassInstanceDescriptor->Name 
            = strdup("T5's Second Order Low Pass");
        soLowpassInstanceDescriptor->Maker
            = strdup("Juergen Herrmann (t-5@t-5.eu)");
        soLowpassInstanceDescriptor->Copyright
            = strdup("3-clause BSD licence");
        soLowpassInstanceDescriptor->PortCount
            = PORTCOUNT;
        portDescriptors
            = (LADSPA_PortDescriptor *)calloc(PORTCOUNT, sizeof(LADSPA_PortDescriptor));
        soLowpassInstanceDescriptor->PortDescriptors
            = (const LADSPA_PortDescriptor *)portDescriptors;
        portDescriptors[SF_INPUT]
            = LADSPA_PORT_INPUT | LADSPA_PORT_AUDIO;
        portDescriptors[SF_OUTPUT]
            = LADSPA_PORT_OUTPUT | LADSPA_PORT_AUDIO;
        portDescriptors[SF_F]
            = LADSPA_PORT_INPUT | LADSPA_PORT_CONTROL;
        portDescriptors[SF_GAIN]
            = LADSPA_PORT_INPUT | LADSPA_PORT_CONTROL;
        portDescriptors[SF_MMAPFNAME]
            = LADSPA_PORT_INPUT | LADSPA_PORT_CONTROL;
        portNames
            = (char **)calloc(PORTCOUNT, sizeof(char *));
        soLowpassInstanceDescriptor->PortNames 
            = (const char **)portNames;
        portNames[SF_INPUT]
            = strdup("Input");
        portNames[SF_OUTPUT]
            = strdup("Output");
        portNames[SF_F]
            = strdup("Cutoff Frequency [Hz]");
        portNames[SF_GAIN]
            = strdup("Overall Gain [dB]");
        portNames[SF_MMAPFNAME]
            = strdup("MMAP-Filename-Part");
        portRangeHints = ((LADSPA_PortRangeHint *)
            calloc(PORTCOUNT, sizeof(LADSPA_PortRangeHint)));
        soLowpassInstanceDescriptor->PortRangeHints
            = (const LADSPA_PortRangeHint *)portRangeHints;
        portRangeHints[SF_F].HintDescriptor
            = (LADSPA_HINT_BOUNDED_BELOW 
            | LADSPA_HINT_BOUNDED_ABOVE
            | LADSPA_HINT_SAMPLE_RATE
            | LADSPA_HINT_LOGARITHMIC
            | LADSPA_HINT_DEFAULT_440);
        portRangeHints[SF_F].LowerBound 
            = 0;
        portRangeHints[SF_F].UpperBound
            = 0.5;
        // Gain ------------------------------------------------------------ */
        portRangeHints[SF_GAIN].HintDescriptor
            = (LADSPA_HINT_BOUNDED_BELOW 
            | LADSPA_HINT_BOUNDED_ABOVE
            | LADSPA_HINT_DEFAULT_0);
        portRangeHints[SF_GAIN].LowerBound 
            = -20;
        portRangeHints[SF_GAIN].UpperBound
            = 20;
        // MMAP Filename --------------------------------------------------- */
        portRangeHints[SF_MMAPFNAME].HintDescriptor
            = (LADSPA_HINT_BOUNDED_BELOW 
            | LADSPA_HINT_BOUNDED_ABOVE
            | LADSPA_HINT_DEFAULT_0);
        portRangeHints[SF_MMAPFNAME].LowerBound 
            = 0;
        portRangeHints[SF_MMAPFNAME].UpperBound
            = 10000000000;
        // In- and Outpus -------------------------------------------------- */
        portRangeHints[SF_INPUT].HintDescriptor
            = 0;
        portRangeHints[SF_OUTPUT].HintDescriptor
            = 0;
        soLowpassInstanceDescriptor->instantiate 
            = instantiateSoLowHighPass;
        soLowpassInstanceDescriptor->connect_port 
            = connectPortToSoLowHighPass;
        soLowpassInstanceDescriptor->activate
            = activateSoLowHighPass;
        soLowpassInstanceDescriptor->run
            = runSoLowpass;
        soLowpassInstanceDescriptor->run_adding
            = NULL;
        soLowpassInstanceDescriptor->set_run_adding_gain
            = NULL;
        soLowpassInstanceDescriptor->deactivate
            = NULL;
        soLowpassInstanceDescriptor->cleanup
            = cleanupSoLowpass;
    }
}
  

/* called automatically when the library is unloaded. */
void plugin_deinit() {
    deleteDescriptor(soLowpassInstanceDescriptor);
}


/* Return a descriptor of the requested plugin types. */
const LADSPA_Descriptor * ladspa_descriptor(unsigned long index) {
    /* Return the requested descriptor or null if the index is out of range. */
    switch (index) {
    case 0:
        return soLowpassInstanceDescriptor;
    default:
        return NULL;
    }
}
