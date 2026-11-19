/* runbiquad.h

   Free software by Juergen Herrmann, t-5@t-5.eu. Do with it, whatever you
   want. No warranty. None, whatsoever. Also see license.txt .

*/

/*****************************************************************************/

#include <math.h>
#include <ladspa.h>
#include "coeffs.h"


// return type for the runXxxBiQuad() functions
typedef struct {
    LADSPA_Data xprev1;
    LADSPA_Data yprev1;
    LADSPA_Data xprev2;
    LADSPA_Data yprev2;
    LADSPA_Data xprev3;
    LADSPA_Data yprev3;
} RunBiquadPrevSamples;


void run1stOrderBiquad(LADSPA_Data *buf, int sampleCount, BiquadCoeffs coeffs, int runTimes, RunBiquadPrevSamples* prevSamples);
void run1stOrderBiquad(LADSPA_Data *buf, int sampleCount, BiquadCoeffs coeffs, int runTimes, RunBiquadPrevSamples* prevSamples) {
    unsigned long sampleIndex;
    LADSPA_Data x, y;           // currently processed input/output samples.
    LADSPA_Data xprev1, yprev1; // previously processed input/output samples.
    LADSPA_Data* mybuf;

    for (int i = 0; i < runTimes; i++) {
        xprev1 = (*prevSamples).xprev1;
        yprev1 = (*prevSamples).yprev1;
        mybuf = buf;
        for (sampleIndex = 0; sampleIndex < sampleCount; sampleIndex++) {
            x = *(mybuf);
            y = (coeffs.b0 * x + coeffs.b1 * xprev1 - coeffs.a1 * yprev1);
            xprev1 = x;
            yprev1 = y;
            *(mybuf++) = y;
        }
        (*prevSamples).xprev1 = xprev1;
        (*prevSamples).yprev1 = yprev1;
        prevSamples++;
    }
}


void run2ndOrderBiquad(LADSPA_Data *buf, int sampleCount, BiquadCoeffs coeffs, int runTimes, RunBiquadPrevSamples* prevSamples);
void run2ndOrderBiquad(LADSPA_Data *buf, int sampleCount, BiquadCoeffs coeffs, int runTimes, RunBiquadPrevSamples* prevSamples) {
    unsigned long sampleIndex;
    LADSPA_Data x, y;           // currently processed input/output samples.
    LADSPA_Data xprev1, yprev1; // previously processed input/output samples.
    LADSPA_Data xprev2, yprev2; // even more previously processed input/output samples.
    LADSPA_Data* mybuf;

    for (int i = 0; i < runTimes; i++) {
        xprev1 = (*prevSamples).xprev1;
        yprev1 = (*prevSamples).yprev1;
        xprev2 = (*prevSamples).xprev2;
        yprev2 = (*prevSamples).yprev2;
        mybuf = buf;
        for (sampleIndex = 0; sampleIndex < sampleCount; sampleIndex++) {
            x = *(mybuf);
            y = (coeffs.b0 * x + coeffs.b1 * xprev1 + coeffs.b2 * xprev2 - 
                                 coeffs.a1 * yprev1 - coeffs.a2 * yprev2);
            xprev2 = xprev1;
            xprev1 = x;
            yprev2 = yprev1;
            yprev1 = y;
            *(mybuf++) = y;
        }
        (*prevSamples).xprev1 = xprev1;
        (*prevSamples).yprev1 = yprev1;
        (*prevSamples).xprev2 = xprev2;
        (*prevSamples).yprev2 = yprev2;
        prevSamples++;
    }
}


void run3rdOrderBiquad(LADSPA_Data *buf, int sampleCount, BiquadCoeffs coeffs, int runTimes, RunBiquadPrevSamples* prevSamples);
void run3rdOrderBiquad(LADSPA_Data *buf, int sampleCount, BiquadCoeffs coeffs, int runTimes, RunBiquadPrevSamples* prevSamples) {
    unsigned long sampleIndex;
    LADSPA_Data x, y;           // currently processed input/output samples.
    LADSPA_Data xprev1, yprev1; // previously processed input/output samples.
    LADSPA_Data xprev2, yprev2; // even more previously processed input/output samples.
    LADSPA_Data xprev3, yprev3; // even lots more previously processed input/output samples.
    LADSPA_Data* mybuf;

    for (int i = 0; i < runTimes; i++) {
        xprev1 = (*prevSamples).xprev1;
        yprev1 = (*prevSamples).yprev1;
        xprev2 = (*prevSamples).xprev2;
        yprev2 = (*prevSamples).yprev2;
        xprev3 = (*prevSamples).xprev3;
        yprev3 = (*prevSamples).yprev3;
        mybuf = buf;
        for (sampleIndex = 0; sampleIndex < sampleCount; sampleIndex++) {
            x = *(mybuf);
            y = (coeffs.b0 * x + coeffs.b1 * xprev1 + coeffs.b2 * xprev2 + coeffs.b3 * xprev3 - 
                                 coeffs.a1 * yprev1 - coeffs.a2 * yprev2 - coeffs.a3 * yprev3);
            xprev3 = xprev2;
            xprev2 = xprev1;
            xprev1 = x;
            yprev3 = yprev2;
            yprev2 = yprev1;
            yprev1 = y;
            *(mybuf++) = y;
        }
        (*prevSamples).xprev1 = xprev1;
        (*prevSamples).yprev1 = yprev1;
        (*prevSamples).xprev2 = xprev2;
        (*prevSamples).yprev2 = yprev2;
        (*prevSamples).xprev3 = xprev3;
        (*prevSamples).yprev3 = yprev3;
        prevSamples++;
    }
}
