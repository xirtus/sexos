/* secondorder.h

    Functions for Second Order Butterworth (Low|High)Pass

    Free software by Juergen Herrmann, t-5@t-5.eu. Do with it, whatever you
    want. No warranty. None, whatsoever. Also see license.txt .

*/


#include "runbiquad.h"


#define SF_INPUT       0
#define SF_OUTPUT      1
#define SF_F           2
#define SF_GAIN        3
#define SF_MMAPFNAME   4
#define PORTCOUNT      5


/* Instance data for the Second Order (Low|High)Pass filter */
typedef struct {
    long created_ns;
    time_t created_s;
    LADSPA_Data sampleRate;
    LADSPA_Data* mmapArea;
    LADSPA_Data* mmapFname;
    // port pointers
    LADSPA_Data* input;
    LADSPA_Data* output;
    LADSPA_Data* f;
    LADSPA_Data* gain;
    // storage for previously processed samples
    RunBiquadPrevSamples* prevSamples;
} SoLowHighPass;


/* Construct a new Second Order (Low|High)Pass plugin instance. */
LADSPA_Handle instantiateSoLowHighPass(const LADSPA_Descriptor* descriptor,
                                       unsigned long sampleRate) {
    SoLowHighPass* instance;
    instance = (SoLowHighPass*)malloc(sizeof(SoLowHighPass));
    if (instance) {
        instance->sampleRate = (LADSPA_Data)sampleRate;
        instance->mmapArea = NULL;
        instance->prevSamples = malloc(sizeof(RunBiquadPrevSamples));
    }  
    return instance;
}


/* Initialise and activate a SecondOrder(Low|High)Pass plugin instance. */
void activateSoLowHighPass(LADSPA_Handle ladspaHandle);
void activateSoLowHighPass(LADSPA_Handle ladspaHandle) {
    SoLowHighPass* instance;
    instance = (SoLowHighPass*)ladspaHandle;
    instance->prevSamples->xprev1 = 0;
    instance->prevSamples->yprev1 = 0;
    instance->prevSamples->xprev2 = 0;
    instance->prevSamples->yprev2 = 0;
}


/* Connect a SecondOrder(Low|High)Pass port to a data location.  */
void connectPortToSoLowHighPass(LADSPA_Handle ladspaHandle,
                                unsigned long port,
                                LADSPA_Data* dataLocation);
void connectPortToSoLowHighPass(LADSPA_Handle ladspaHandle,
                                unsigned long port,
                                LADSPA_Data* dataLocation) {
  
    SoLowHighPass* instance;
    instance = (SoLowHighPass*)ladspaHandle;

    switch (port) {
    case SF_INPUT:
        instance->input = dataLocation;
        break;
    case SF_OUTPUT:
        instance->output = dataLocation;
        break;
    case SF_F:
        instance->f = dataLocation;
        break;
    case SF_GAIN:
        instance->gain = dataLocation;
        break;
    case SF_MMAPFNAME:
        instance->mmapFname = dataLocation;
        break;
    }
}


void runSoBiquad(LADSPA_Handle ladspaHandle, unsigned long sampleCount, BiquadCoeffs coeffs);
void runSoBiquad(LADSPA_Handle ladspaHandle, unsigned long sampleCount, BiquadCoeffs coeffs) {
    SoLowHighPass* instance;
    LADSPA_Data* input;
    LADSPA_Data* output;
    LADSPA_Data unchanged = 0.0;
    LADSPA_Data changed;
    LADSPA_Data* mmptr;
    unsigned long sampleIndex;
    LADSPA_Data gainFactor;

    instance = (SoLowHighPass*)ladspaHandle;
    input = instance->input;
    output = instance->output;
    // memcpy parameters over from mmapped area
    mmptr = instance->mmapArea;
    if (mmptr != NULL) {
        memcpy(&changed, mmptr, sizeof(LADSPA_Data));
        if (changed != 0.0) {
            mmptr += 1;
            memcpy(instance->f, mmptr, sizeof(LADSPA_Data));
            mmptr += 1;
            memcpy(instance->gain, mmptr, sizeof(LADSPA_Data));
        }
        // reset changed flag to re-enable parametrization by control inputs
        memcpy(instance->mmapArea, &unchanged, sizeof(LADSPA_Data));
    }
    // apply gain if gainFactor != 1, in any case make sure output buf is filled
    // as it will be used by the run2ndOrderBiquad() function.
    gainFactor = dbToGainFactor(*(instance->gain));
    if (gainFactor == 1.0) {
        for (sampleIndex = 0; sampleIndex < sampleCount; sampleIndex++) {
            *(output++) = *(input++);
        }
    } else {
        for (sampleIndex = 0; sampleIndex < sampleCount; sampleIndex++) {
            *(output++) = *(input++) * gainFactor;
        }
    }
    run2ndOrderBiquad(instance->output, sampleCount, coeffs, 1, instance->prevSamples);
}
