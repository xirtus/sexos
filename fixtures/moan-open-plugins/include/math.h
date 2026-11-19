/* SexOS plugin C surface: the math functions in crates/moan-plugin/src/imports.rs. */
#ifndef SEXOS_PLUGIN_MATH_H
#define SEXOS_PLUGIN_MATH_H
#define M_PI 3.14159265358979323846
#define M_PI_2 1.57079632679489661923
#define M_LN2 0.69314718055994530942
#define M_LN10 2.30258509299404568402
#define M_SQRT2 1.41421356237309504880
#define HUGE_VAL __builtin_huge_val()
#define HUGE_VALF __builtin_huge_valf()
#define INFINITY __builtin_inff()
#define NAN __builtin_nanf("")
#define isnan(x) __builtin_isnan(x)
#define isinf(x) __builtin_isinf(x)
#define isfinite(x) __builtin_isfinite(x)
double sin(double x);
float sinf(float x);
double cos(double x);
float cosf(float x);
double tan(double x);
float tanf(float x);
double asin(double x);
float asinf(float x);
double acos(double x);
float acosf(float x);
double atan(double x);
float atanf(float x);
double sinh(double x);
float sinhf(float x);
double cosh(double x);
float coshf(float x);
double tanh(double x);
float tanhf(float x);
double exp(double x);
float expf(float x);
double exp2(double x);
float exp2f(float x);
double expm1(double x);
float expm1f(float x);
double log(double x);
float logf(float x);
double log2(double x);
float log2f(float x);
double log10(double x);
float log10f(float x);
double log1p(double x);
float log1pf(float x);
double sqrt(double x);
float sqrtf(float x);
double cbrt(double x);
float cbrtf(float x);
double fabs(double x);
float fabsf(float x);
double floor(double x);
float floorf(float x);
double ceil(double x);
float ceilf(float x);
double round(double x);
float roundf(float x);
double trunc(double x);
float truncf(float x);
double rint(double x);
float rintf(float x);
double pow(double x, double y);
float powf(float x, float y);
double atan2(double x, double y);
float atan2f(float x, float y);
double fmod(double x, double y);
float fmodf(float x, float y);
double hypot(double x, double y);
float hypotf(float x, float y);
double fmin(double x, double y);
float fminf(float x, float y);
double fmax(double x, double y);
float fmaxf(float x, float y);
long lrint(double x);
long lrintf(float x);
void sincosf(float x, float* s, float* c);
#endif
