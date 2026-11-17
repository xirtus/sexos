//! SexOS wide-gamut linear-light color, HDR transfer, LUT, physical film emulation,
//! 4-way color grading, and real-time broadcast video scopes engine.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod film;
pub mod frame;
pub mod lut;
pub mod scopes;
pub mod space;
pub mod wheels;

pub use film::{cineon_log_to_linear, linear_to_cineon_log, FilmParams, FilmStock};
pub use frame::{
    detect_nv12_range, FrameError, LinearFrame, RangeDetection, RgbaF32, YuvMatrix, YuvRange,
};
pub use lut::{CubeLut, InterpMode, Lut1D, Lut3D, LutError};
pub use scopes::{
    compute_histogram, HistogramData, ScopeInput, ScopeStandard, ScopeSurface, WaveformMode,
};
pub use space::{
    convert_primaries, convert_rgb, hlg_eotf, hlg_inverse_oetf, hlg_oetf, hlg_system_gamma,
    hlg_to_linear_nits, linear_nits_to_hlg, linear_rec2020_to_oklab, linear_rec2020_to_rec709,
    linear_rec2020_to_xyz, linear_rec709_to_oklab, linear_rec709_to_rec2020, linear_rec709_to_xyz,
    linear_to_rec2020, linear_to_rec601, linear_to_rec709, linear_to_srgb, mul_mat3,
    oklab_to_linear_rec2020, oklab_to_linear_rec709, pq_eotf, pq_eotf_nits, pq_oetf, pq_oetf_nits,
    primaries_matrix, rec2020_to_linear, rec601_to_linear, rec709_to_linear, sdr_on_hdr_scale,
    srgb_to_linear, tone_map, tone_map_rgb, xyz_to_linear_rec2020, xyz_to_linear_rec709,
    ColorPrimaries, ColorSpace, ToneMapConfig, ToneMapMode, BT2408_SDR_WHITE_NITS,
    HLG_NOMINAL_PEAK_NITS, PQ_PEAK_NITS, REC2020_TO_REC601, REC2020_TO_REC709, REC2020_TO_XYZ,
    REC601_TO_REC2020, REC601_TO_REC709, REC601_TO_XYZ, REC709_TO_REC2020, REC709_TO_REC601,
    REC709_TO_XYZ, REF_WHITE_NITS, XYZ_TO_REC2020, XYZ_TO_REC601, XYZ_TO_REC709,
};
pub use wheels::{ColorGrade, Curve1D};
