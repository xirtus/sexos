/* coeffs.h

   Free software by Juergen Herrmann, t-5@t-5.eu. Do with it, whatever you
   want. No warranty. None, whatsoever. Also see license.txt .

*/

/*****************************************************************************/


#include <complex.h>


/* biquad coefficients for up to 3rd order filters, not all coefficients may
   actually be in use in the code (1st/2nd order filters) */
typedef struct {
  float a1;
  float a2;
  float a3;
  float b0;
  float b1;
  float b2;
  float b3;
} BiquadCoeffs;


/* complex array helpers =================================================== */

/* convolve two arrays of complex numbers, both max len of 5 */
void convolve(float complex signal[5], int signal_len,
              float complex kernel[5], int kernel_len,
              float complex * result) {
    float complex ret[9];
    for (int n=0; n<(signal_len + kernel_len - 1); n++) {
        int kmin, kmax;
        ret[n] = 0;
        kmin = (n >= kernel_len - 1) ? n - (kernel_len - 1) : 0;
        kmax = (n < signal_len - 1) ? n : signal_len - 1;
        for (int k=kmin; k<=kmax; k++)
            ret[n] += signal[k] * kernel[n - k];
    }
    for (int n=0; n<(signal_len + kernel_len - 1); n++) {
        *result = ret[n];
        result++;
    }
}

/* find coefficients of polynomial for given zeroes
   (third order polynomial => array of three zeros!) */
void poly(float complex seq_of_zeros[3], float complex* ret) {
    int signal_len = 1;
    float complex signal[4] = { 1, 0, 0, 0 };
    float complex kernel[2] = { 1, 0 };
    for (int k=0; k<3; k++) {
        kernel[1] = -1.0 * seq_of_zeros[k];
        convolve(signal, signal_len, kernel, 2, (float complex *)&signal);
        signal_len++;
    }
    for (int i=0; i<4; i++) {
        *ret = creal(signal[i]);
        ret++;
    }
}

/* return 3rd order butterworth filter coefficients,
    Wn is the cutoff frequency in multiples of the nyquist frequency,
    btype can either be "low" or "high"
*/
BiquadCoeffs butter_3rd(float Wn, char* btype) {
    BiquadCoeffs coeffs = { 0, 0, 0, 0, 0, 0, 0 };
    int idx;
    float complex p_proto[3];    // array of the three analog prototype poles
    float warped;                // warped cutoff frequency
    float complex z_analog[3];   // array of the three analog zeroes
    float complex z_digital[3];  // array of the three digital zeroes
    float complex p_analog[3];   // array of the three analog poles
    float complex p_digital[3];  // array of the three digital poles
    float k_analog, k_digital;   // analog and digital gain
    float complex tmp1, tmp2;    // temporary variables
    float complex a[4], b[4];    // b and a arrays for return values
    // calculate poles of the analog prototype filter
    for (int m = -2; m <= 2; m += 2){
        idx = (m + 2) / 2;
        p_proto[idx] = -1.0 * cexp(0+1i * M_PI * m / 6.0);
    }
    // pre-warping the cutoff frequency
    warped = 4.0 * tan(M_PI * Wn / 2.0);
    // construct high/lowpass zeros and poles according btype
    if (strcmp(btype, "low") == 0) {
        for (idx=0; idx<3; idx++) {
            z_digital[idx] = -1.0;
            p_analog[idx] = warped * p_proto[idx];
        }
        k_analog = warped * warped * warped;
    } else if (strcmp(btype, "high") == 0) {
        for (idx=0; idx<3; idx++) {
            z_analog[idx] = 0.0;
            z_digital[idx] = 1.0;
            p_analog[idx] = warped / p_proto[idx];
        }
        k_analog = 1;
    }
    // calculate p_digital (poles of the digital filter)
    for (idx=0; idx<3; idx++)
        p_digital[idx] = (4 + p_analog[idx]) / (4 - p_analog[idx]);
    // calculate k_digital (digital gain)
    if (strcmp(btype, "low") == 0) {
        tmp1 = 1.0; // special case, z_analog is empty in case of lowpass filter
    } else if (strcmp(btype, "high") == 0) {
        tmp1 = 4.0 - z_analog[0];
        for (idx=1; idx<3; idx++)
            tmp1 = tmp1 * (4.0 - z_analog[idx]);
    }
    tmp2 = 4.0 - p_analog[0];
    for (idx=1; idx<3; idx++)
        tmp2 = tmp2 * (4.0 - p_analog[idx]);
    k_digital = k_analog * creal(tmp1 / tmp2);
    // transform to ba form
    poly(z_digital, (float complex *)&b);
    for (idx=0; idx<4; idx++)
        b[idx] = k_digital * b[idx];
    poly(p_digital, (float complex *)&a);
    // stuff the results into the return variable
    coeffs.a1 = a[1];
    coeffs.a2 = a[2];
    coeffs.a3 = a[3];
    coeffs.b0 = b[0];
    coeffs.b1 = b[1];
    coeffs.b2 = b[2];
    coeffs.b3 = b[3];
    return coeffs;
}


/* Actual coefficient calculation functions ================================ */

BiquadCoeffs calcCoeffs1stOrderLowpass(float f, float samplerate) {
    BiquadCoeffs coeffs;
    float w = 2 * M_PI * f;
    float A = 1 / (tan((w / samplerate) / 2));
    coeffs.b0 = 1 / (1 + A);
    coeffs.b1 = coeffs.b0;
    coeffs.a1 = (1 - A) / (1 + A);
    return coeffs;
}


BiquadCoeffs calcCoeffs1stOrderHighpass(float f, float samplerate) {
    BiquadCoeffs coeffs;
    float w = 2 * M_PI * f;
    float A = 1 / (tan((w / samplerate) / 2));
    coeffs.b0 = A / (1 + A);
    coeffs.b1 = -1 * coeffs.b0;
    coeffs.a1 = (1 - A) / (1 + A);
    return coeffs;
}


BiquadCoeffs calcCoeffs2ndOrderLowpass(float f, float q, float samplerate) {
    BiquadCoeffs coeffs;
    float w0 = 2 * M_PI * f / samplerate;
    float alpha = sin(w0) / 2 / q;
    float cs = cos(w0);
    float norm = 1 / (1 + alpha);
    coeffs.b0 = (1 - cs) / 2 * norm;
    coeffs.b1 = (1 - cs) * norm;
    coeffs.b2 = coeffs.b0;
    coeffs.a1 = -2 * cs * norm;
    coeffs.a2 = (1 - alpha) * norm;
    return coeffs;
}


BiquadCoeffs calcCoeffs2ndOrderHighpass(float f, float q, float samplerate) {
    BiquadCoeffs coeffs;
    float w0 = 2 * M_PI * f / samplerate;
    float alpha = sin(w0) / 2 /q;
    float cs = cos(w0);
    float norm = 1 / (1 + alpha);
    coeffs.b0 = (1 + cs) / 2 * norm;
    coeffs.b1 = -1.0 * (1 + cs) * norm;
    coeffs.b2 = coeffs.b0;
    coeffs.a1 = -2 * cs * norm;
    coeffs.a2 = (1 - alpha) * norm;
    return coeffs;
}


BiquadCoeffs calcCoeffs3rdOrderLowpass(float f, float samplerate) {
    return butter_3rd(2 * f / samplerate, "low");
}


BiquadCoeffs calcCoeffs3rdOrderHighpass(float f, float samplerate) {
    return butter_3rd(2 * f / samplerate, "high");
}


BiquadCoeffs calcCoeffsLowShelf(float f, float g, float q, float samplerate) {
    BiquadCoeffs coeffs;
    float w0 = 2.0 * M_PI * f / samplerate;
    float alpha = sin(w0) / (2.0 * q);
    float A = pow(10, g / 40.0);
    float cs = cos(w0);
    float norm = 1 / ((A+1.0) + (A-1.0)*cs + 2.0*sqrt(A)*alpha);
    coeffs.b0 = norm * (    A*( (A+1.0) - (A-1.0)*cs + 2.0*sqrt(A)*alpha ));
    coeffs.b1 = norm * (2.0*A*( (A-1.0) - (A+1.0)*cs                     ));
    coeffs.b2 = norm * (    A*( (A+1.0) - (A-1.0)*cs - 2.0*sqrt(A)*alpha ));
    coeffs.a1 = norm * ( -2.0*( (A-1.0) + (A+1.0)*cs                     ));
    coeffs.a2 = norm * (        (A+1.0) + (A-1.0)*cs - 2.0*sqrt(A)*alpha);
    return coeffs;
}


BiquadCoeffs calcCoeffsPeaking(float f, float g, float q, float samplerate) {
    BiquadCoeffs coeffs;
    float w0 = 2.0 * M_PI * f / samplerate;
    float alpha = sin(w0) / (2.0 * q);
    float A = pow(10, g / 40.0);
    float cs = cos(w0);
    float norm = 1 / (1.0 + alpha / A);
    coeffs.b0 = norm * (1.0 + alpha * A);
    coeffs.b1 = norm * (-2.0 * cs);
    coeffs.b2 = norm * (1.0 - alpha * A);
    coeffs.a1 = norm * (-2.0 * cs);
    coeffs.a2 = norm * (1.0 - alpha / A);
    return coeffs;
}


BiquadCoeffs calcCoeffsHighShelf(float f, float g, float q, float samplerate) {
    BiquadCoeffs coeffs;
    float w0 = 2.0 * M_PI * f / samplerate;
    float alpha = sin(w0) / (2.0 * q);
    float A = pow(10, g / 40.0);
    float cs = cos(w0);
    float norm = 1 / ((A+1.0) - (A-1.0)*cs + 2.0*sqrt(A)*alpha);
    coeffs.b0 = norm * (     A*( (A+1.0) + (A-1.0)*cs + 2.0*sqrt(A)*alpha ));
    coeffs.b1 = norm * (-2.0*A*( (A-1.0) + (A+1.0)*cs                     ));
    coeffs.b2 = norm * (     A*( (A+1.0) + (A-1.0)*cs - 2.0*sqrt(A)*alpha ));
    coeffs.a1 = norm * (   2.0*( (A-1.0) - (A+1.0)*cs                     ));
    coeffs.a2 = norm * (         (A+1.0) - (A-1.0)*cs - 2.0*sqrt(A)*alpha);
    return coeffs;
}


BiquadCoeffs calcCoeffsLinkwitzTransform(float f, float q, float f0, float q0, float samplerate) {
    BiquadCoeffs coeffs;

    // analog coefficients:
    float fc = 1;
    float d0i = pow(2 * M_PI * f0, 2);
    float d1i = (2 * M_PI * f0) / q0;
    float d2i = 1;
    float c0i = pow(2 * M_PI * f, 2);
    float c1i = (2 * M_PI * f) / q;
    float c2i = 1;
    float gn = (2 * M_PI * fc) / (tan(M_PI * fc / samplerate));
    float cci = c0i + gn * c1i + gn * gn * c2i;

    coeffs.a1 = 2 * (c0i - gn * gn * c2i) / cci;
    coeffs.a2 = (c0i -gn * c1i + gn * gn * c2i) / cci;
    coeffs.b0 = (d0i + gn * d1i + gn * gn * d2i) / cci;
    coeffs.b1 = 2 * (d0i - (gn * gn * d2i)) / cci;
    coeffs.b2 = (d0i - gn * d1i + gn * gn * d2i) / cci;

    return coeffs;

}