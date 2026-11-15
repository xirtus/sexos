// SPDX-License-Identifier: GPL-3.0-only
// Original developer-only CLAP boundary probes. Not a production effect.
#include <clap/clap.h>
#ifndef RIBBON_MODE
#error RIBBON_MODE required
#endif
static bool eq(const char *a, const char *b) {
    while (*a && *a == *b) { ++a; ++b; }
    return *a == *b;
}
static const char *features[] = {CLAP_PLUGIN_FEATURE_AUDIO_EFFECT, CLAP_PLUGIN_FEATURE_STEREO, 0};
static const clap_plugin_descriptor_t descriptor = {
    .clap_version=CLAP_VERSION_INIT, .id="sexos.ribbon.boundary", .name="Ribbon Boundary",
    .vendor="SexOS developer qualification", .version="1", .features=features,
};
static bool init(const clap_plugin_t *p) { (void)p; return true; }
static void nothing(const clap_plugin_t *p) { (void)p; }
static bool activate(const clap_plugin_t *p,double rate,uint32_t min,uint32_t max) {
    (void)p; return rate == 48000 && min >= 1 && max <= 256;
}
static uint32_t ports(const clap_plugin_t *p,bool input) { (void)p; (void)input; return 1; }
static bool port(const clap_plugin_t *p,uint32_t index,bool input,clap_audio_port_info_t *info) {
    (void)p; (void)input; if (index) return false;
    info->id=0; info->name[0]='R'; info->name[1]=0; info->flags=CLAP_AUDIO_PORT_IS_MAIN;
    info->channel_count=2; info->port_type=CLAP_PORT_STEREO; info->in_place_pair=CLAP_INVALID_ID;
    return true;
}
static const clap_plugin_audio_ports_t audio_ports={ports,port};
static const void *extension(const clap_plugin_t *p,const char *id) {
    (void)p; return eq(id,CLAP_EXT_AUDIO_PORTS) ? &audio_ports : 0;
}
static clap_process_status process(const clap_plugin_t *p,const clap_process_t *b) {
    (void)p;
    if (!b || b->audio_inputs_count != 1 || b->audio_outputs_count != 1)
        return CLAP_PROCESS_ERROR;
#if RIBBON_MODE == 1
    __builtin_trap();
#elif RIBBON_MODE == 2
    for (;;) __asm__ volatile("pause");
#elif RIBBON_MODE == 3 || RIBBON_MODE == 4
    // Address is test data in sixteen exact finite nibble-valued samples, never
    // a newly invented production pointer parameter. Parent keeps the real page
    // mapped until this exact helper run is fully retired.
    if (b->frames_count < 16) return CLAP_PROCESS_ERROR;
    uintptr_t address=0;
    for (uint32_t i=0;i<16;i++) {
        float v=b->audio_inputs[0].data32[0][i];
        if (v != v || v < 0 || v > 15) return CLAP_PROCESS_ERROR;
        uint32_t n=(uint32_t)v;
        if (v != (float)n) return CLAP_PROCESS_ERROR;
        address |= (uintptr_t)n << (i*4);
    }
    if (!address || (address & 4095)) return CLAP_PROCESS_ERROR;
#if RIBBON_MODE == 3
    volatile unsigned char observed=*(volatile unsigned char *)address;
    (void)observed;
#else
    *(volatile unsigned char *)address=0xa5;
#endif
    // If forbidden access succeeds, return a distinctive finite PCM sentinel;
    // never turn a failed isolation probe into a normal processing result.
    for (uint32_t i=0;i<b->frames_count;i++) {
        b->audio_outputs[0].data32[0][i]=13.0f;
        b->audio_outputs[0].data32[1][i]=11.0f;
    }
    return CLAP_PROCESS_CONTINUE;
#endif
    for (uint32_t i=0;i<b->frames_count;i++) {
        b->audio_outputs[0].data32[0][i]=b->audio_inputs[0].data32[1][i];
        b->audio_outputs[0].data32[1][i]=b->audio_inputs[0].data32[0][i];
    }
    return CLAP_PROCESS_CONTINUE;
}
static const clap_plugin_t plugin={
 .desc=&descriptor,.init=init,.destroy=nothing,.activate=activate,.deactivate=nothing,
 .start_processing=init,.stop_processing=nothing,.reset=nothing,.process=process,
 .get_extension=extension,.on_main_thread=nothing,
};
static uint32_t count(const clap_plugin_factory_t *f) { (void)f; return 1; }
static const clap_plugin_descriptor_t *describe(const clap_plugin_factory_t *f,uint32_t i) {
 (void)f; return i ? 0 : &descriptor;
}
static const clap_plugin_t *create(const clap_plugin_factory_t *f,const clap_host_t *h,const char *id) {
 (void)f; return h && h->clap_version.major == 1 && eq(id,descriptor.id) ? &plugin : 0;
}
static const clap_plugin_factory_t factory={count,describe,create};
static bool entry_init(const char *path) { (void)path; return true; }
static void entry_deinit(void) {}
static const void *get_factory(const char *id) { return eq(id,CLAP_PLUGIN_FACTORY_ID) ? &factory : 0; }
CLAP_EXPORT const clap_plugin_entry_t clap_entry={CLAP_VERSION_INIT,entry_init,entry_deinit,get_factory};
