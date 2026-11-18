// SPDX-License-Identifier: GPL-3.0-only
// Crossdress test plugin: a real 64-bit Windows CLAP audio plugin (a DLL named gain.clap),
// built with clang and lld-link against the upstream CLAP headers, no Windows SDK and no
// C runtime. It runs on Windows in any CLAP host too.
//
// "Crossdress Test Gain": stereo in, stereo out, one parameter (Gain, 0..2, default 1)
// that the host may change sample-accurately with CLAP_EVENT_PARAM_VALUE events. Its
// instance lives on the process heap (HeapAlloc), DllMain counts attaches, and every
// process() call reports one CLAP_EVENT_PARAM_VALUE back through out_events when the gain
// changed, so the host sees all three directions: host to plugin calls, plugin to host
// callbacks and plugin memory.
//
// The _Static_asserts pin the structure offsets the host side (src/clap.rs) uses.
#include <clap/clap.h>
#include <stddef.h>

typedef void *HANDLE;
typedef int BOOL;
typedef unsigned long DWORD;
__declspec(dllimport) HANDLE __stdcall GetProcessHeap(void);
__declspec(dllimport) void *__stdcall HeapAlloc(HANDLE heap, DWORD flags, size_t bytes);
__declspec(dllimport) BOOL __stdcall HeapFree(HANDLE heap, DWORD flags, void *p);
#define HEAP_ZERO_MEMORY 8

int _fltused = 0;

_Static_assert(offsetof(clap_plugin_entry_t, init) == 16, "entry.init");
_Static_assert(offsetof(clap_plugin_entry_t, get_factory) == 32, "entry.get_factory");
_Static_assert(offsetof(clap_plugin_factory_t, create_plugin) == 16, "factory.create_plugin");
_Static_assert(offsetof(clap_plugin_descriptor_t, id) == 16, "descriptor.id");
_Static_assert(offsetof(clap_plugin_descriptor_t, name) == 24, "descriptor.name");
_Static_assert(offsetof(clap_plugin_descriptor_t, vendor) == 32, "descriptor.vendor");
_Static_assert(offsetof(clap_plugin_descriptor_t, version) == 64, "descriptor.version");
_Static_assert(offsetof(clap_plugin_descriptor_t, features) == 80, "descriptor.features");
_Static_assert(sizeof(clap_host_t) == 88, "host");
_Static_assert(offsetof(clap_host_t, get_extension) == 56, "host.get_extension");
_Static_assert(offsetof(clap_plugin_t, init) == 16, "plugin.init");
_Static_assert(offsetof(clap_plugin_t, process) == 72, "plugin.process");
_Static_assert(offsetof(clap_plugin_t, get_extension) == 80, "plugin.get_extension");
_Static_assert(sizeof(clap_process_t) == 64, "process");
_Static_assert(offsetof(clap_process_t, in_events) == 48, "process.in_events");
_Static_assert(sizeof(clap_audio_buffer_t) == 32, "audio buffer");
_Static_assert(sizeof(clap_input_events_t) == 24, "input events");
_Static_assert(sizeof(clap_output_events_t) == 16, "output events");
_Static_assert(sizeof(clap_event_param_value_t) == 56, "param value event");
_Static_assert(offsetof(clap_event_param_value_t, value) == 48, "param value");
_Static_assert(sizeof(clap_param_info_t) == 1320, "param info");
_Static_assert(offsetof(clap_param_info_t, min_value) == 1296, "param info min");
_Static_assert(offsetof(clap_plugin_params_t, flush) == 40, "params.flush");

static void *zalloc(size_t n) { return HeapAlloc(GetProcessHeap(), HEAP_ZERO_MEMORY, n); }
static void copy_name(char *to, const char *from, size_t n) { size_t i = 0; for (; i + 1 < n && from[i]; i++) to[i] = from[i]; to[i] = 0; }
static int same(const char *a, const char *b) { while (*a && *a == *b) { a++; b++; } return *a == *b; }

static int attaches;
static int entry_inits;

typedef struct {
    clap_plugin_t plugin;
    const clap_host_t *host;
    float gain;
    float reported;
    double sample_rate;
    int active, processing;
} gain_t;

static const char *const features[] = { CLAP_PLUGIN_FEATURE_AUDIO_EFFECT, CLAP_PLUGIN_FEATURE_UTILITY, CLAP_PLUGIN_FEATURE_STEREO, NULL };
static const clap_plugin_descriptor_t descriptor = {
    .clap_version = CLAP_VERSION_INIT,
    .id = "org.sexos.crossdress.test-gain",
    .name = "Crossdress Test Gain",
    .vendor = "SexOS",
    .url = "",
    .manual_url = "",
    .support_url = "",
    .version = "1.0.0",
    .description = "A Windows CLAP plugin that proves Crossdress cells",
    .features = features,
};

static void apply_event(gain_t *g, const clap_event_header_t *h) {
    if (h->space_id != CLAP_CORE_EVENT_SPACE_ID || h->type != CLAP_EVENT_PARAM_VALUE) return;
    const clap_event_param_value_t *v = (const clap_event_param_value_t *)h;
    if (v->param_id != 0) return;
    double x = v->value < 0.0 ? 0.0 : v->value > 2.0 ? 2.0 : v->value;
    g->gain = (float)x;
}

static void report(gain_t *g, const clap_output_events_t *out, uint32_t time) {
    if (!out || g->gain == g->reported) return;
    clap_event_param_value_t e = {0};
    e.header.size = sizeof e;
    e.header.time = time;
    e.header.space_id = CLAP_CORE_EVENT_SPACE_ID;
    e.header.type = CLAP_EVENT_PARAM_VALUE;
    e.param_id = 0;
    e.note_id = -1; e.port_index = -1; e.channel = -1; e.key = -1;
    e.value = g->gain;
    if (out->try_push(out, &e.header)) g->reported = g->gain;
}

static bool g_init(const clap_plugin_t *p) { gain_t *g = p->plugin_data; g->gain = 1.0f; g->reported = 1.0f; return true; }
static void g_destroy(const clap_plugin_t *p) { HeapFree(GetProcessHeap(), 0, p->plugin_data); }
static bool g_activate(const clap_plugin_t *p, double sr, uint32_t min, uint32_t max) {
    gain_t *g = p->plugin_data;
    if (sr < 8000.0 || min > max || max == 0) return false;
    g->sample_rate = sr; g->active = 1; return true;
}
static void g_deactivate(const clap_plugin_t *p) { ((gain_t *)p->plugin_data)->active = 0; }
static bool g_start(const clap_plugin_t *p) { gain_t *g = p->plugin_data; if (!g->active) return false; g->processing = 1; return true; }
static void g_stop(const clap_plugin_t *p) { ((gain_t *)p->plugin_data)->processing = 0; }
static void g_reset(const clap_plugin_t *p) { (void)p; }

static clap_process_status g_process(const clap_plugin_t *p, const clap_process_t *proc) {
    gain_t *g = p->plugin_data;
    if (!g->processing || proc->audio_inputs_count != 1 || proc->audio_outputs_count != 1) return CLAP_PROCESS_ERROR;
    const clap_audio_buffer_t *in = &proc->audio_inputs[0];
    const clap_audio_buffer_t *out = &proc->audio_outputs[0];
    if (in->channel_count != 2 || out->channel_count != 2 || !in->data32 || !out->data32) return CLAP_PROCESS_ERROR;
    uint32_t n = proc->frames_count;
    uint32_t events = proc->in_events ? proc->in_events->size(proc->in_events) : 0;
    uint32_t e = 0;
    for (uint32_t i = 0; i < n; i++) {
        // Sample accurate: every event due at or before frame i applies before it.
        while (e < events) {
            const clap_event_header_t *h = proc->in_events->get(proc->in_events, e);
            if (!h || h->time > i) break;
            apply_event(g, h);
            report(g, proc->out_events, i);
            e++;
        }
        for (uint32_t c = 0; c < 2; c++) out->data32[c][i] = in->data32[c][i] * g->gain;
    }
    // Events past the block's end still apply (a host bug, but never dropped).
    for (; e < events; e++) { apply_event(g, proc->in_events->get(proc->in_events, e)); report(g, proc->out_events, n ? n - 1 : 0); }
    return CLAP_PROCESS_CONTINUE;
}

static uint32_t params_count(const clap_plugin_t *p) { (void)p; return 1; }
static bool params_info(const clap_plugin_t *p, uint32_t index, clap_param_info_t *info) {
    (void)p;
    if (index != 0) return false;
    info->id = 0;
    info->flags = CLAP_PARAM_IS_AUTOMATABLE;
    info->cookie = NULL;
    copy_name(info->name, "Gain", sizeof info->name);
    copy_name(info->module, "", sizeof info->module);
    info->min_value = 0.0; info->max_value = 2.0; info->default_value = 1.0;
    return true;
}
static bool params_value(const clap_plugin_t *p, clap_id id, double *out) {
    if (id != 0) return false;
    *out = ((gain_t *)p->plugin_data)->gain;
    return true;
}
static bool params_to_text(const clap_plugin_t *p, clap_id id, double v, char *out, uint32_t n) { (void)p; (void)id; (void)v; (void)out; (void)n; return false; }
static bool params_from_text(const clap_plugin_t *p, clap_id id, const char *t, double *v) { (void)p; (void)id; (void)t; (void)v; return false; }
static void params_flush(const clap_plugin_t *p, const clap_input_events_t *in, const clap_output_events_t *out) {
    gain_t *g = p->plugin_data;
    uint32_t n = in ? in->size(in) : 0;
    for (uint32_t i = 0; i < n; i++) { apply_event(g, in->get(in, i)); report(g, out, 0); }
}
static const clap_plugin_params_t params = { params_count, params_info, params_value, params_to_text, params_from_text, params_flush };

static const void *g_extension(const clap_plugin_t *p, const char *id) { (void)p; return same(id, CLAP_EXT_PARAMS) ? &params : NULL; }
static void g_main_thread(const clap_plugin_t *p) { (void)p; }

static uint32_t f_count(const clap_plugin_factory_t *f) { (void)f; return 1; }
static const clap_plugin_descriptor_t *f_descriptor(const clap_plugin_factory_t *f, uint32_t i) { (void)f; return i == 0 ? &descriptor : NULL; }
static const clap_plugin_t *f_create(const clap_plugin_factory_t *f, const clap_host_t *host, const char *id) {
    (void)f;
    if (!entry_inits || !host || !same(id, descriptor.id) || !clap_version_is_compatible(host->clap_version)) return NULL;
    gain_t *g = zalloc(sizeof *g);
    if (!g) return NULL;
    g->host = host;
    g->plugin.desc = &descriptor;
    g->plugin.plugin_data = g;
    g->plugin.init = g_init;
    g->plugin.destroy = g_destroy;
    g->plugin.activate = g_activate;
    g->plugin.deactivate = g_deactivate;
    g->plugin.start_processing = g_start;
    g->plugin.stop_processing = g_stop;
    g->plugin.reset = g_reset;
    g->plugin.process = g_process;
    g->plugin.get_extension = g_extension;
    g->plugin.on_main_thread = g_main_thread;
    // The host must answer an extension query (a callback into the host).
    (void)host->get_extension(host, CLAP_EXT_PARAMS);
    return &g->plugin;
}
static const clap_plugin_factory_t factory = { f_count, f_descriptor, f_create };

static bool e_init(const char *path) { if (!attaches || !path || !*path) return false; entry_inits++; return true; }
static void e_deinit(void) { entry_inits--; }
static const void *e_factory(const char *id) { return same(id, CLAP_PLUGIN_FACTORY_ID) ? &factory : NULL; }

__declspec(dllexport) const clap_plugin_entry_t clap_entry = { CLAP_VERSION_INIT, e_init, e_deinit, e_factory };

BOOL __stdcall DllMain(HANDLE instance, DWORD reason, void *reserved) {
    (void)instance; (void)reserved;
    if (reason == 1) attaches++;      // DLL_PROCESS_ATTACH
    else if (reason == 0) attaches--; // DLL_PROCESS_DETACH
    return 1;
}
