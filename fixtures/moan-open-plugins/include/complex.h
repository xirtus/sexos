/* Types only: no complex math function is provided, so code using one must be
   unreachable (dropped by --gc-sections) or it fails to load. */
#ifndef SEXOS_PLUGIN_COMPLEX_H
#define SEXOS_PLUGIN_COMPLEX_H
#define complex _Complex
#define I (__extension__ 1.0iF)
#define creal(z) __builtin_creal(z)
#define cimag(z) __builtin_cimag(z)
#define crealf(z) __builtin_crealf(z)
#define cimagf(z) __builtin_cimagf(z)
double _Complex cexp(double _Complex z);
float _Complex cexpf(float _Complex z);
#endif
