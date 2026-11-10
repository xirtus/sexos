#pragma once
#define INFINITY __builtin_inff()
#define NAN __builtin_nanf("")
#define isnan(x) __builtin_isnan(x)
#define signbit(x) __builtin_signbit(x)
#define isinf(x) __builtin_isinf(x)
double ceil(double); float ceilf(float);
double floor(double); float floorf(float);
double trunc(double); float truncf(float);
#define nearbyint sexjs_nearbyint
#define nearbyintf sexjs_nearbyintf
double nearbyint(double); float nearbyintf(float);
double sqrt(double); float sqrtf(float);
double fabs(double); float fabsf(float);
double copysign(double, double); float copysignf(float, float);
