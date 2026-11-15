// SPDX-License-Identifier: GPL-3.0-only
//! Faces-free offline reverse-geocoding gazetteer for Gaze.
//!
//! Resolves EXIF or PNG GPS coordinates `(latitude, longitude)` completely offline
//! inside Gaze's confined domain against a curated global gazetteer of major cities
//! and landmarks across all continents.
//!
//! Privacy guarantee: Gaze performs **zero face detection, zero facial landmarking,
//! zero biometric embedding, and zero cloud reverse-geocoding**.

use alloc::format;
use alloc::string::{String, ToString};

/// Maximum radius in kilometers within which a coordinate snaps to the nearest city.
pub const MAX_CITY_MATCH_KM: f64 = 250.0;

/// Static offline city/landmark entry with coordinates scaled by `1e4` (`0.0001°` ~ 11 m).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CityEntry {
    pub lat_e4: i32,
    pub lon_e4: i32,
    pub city: &'static str,
    pub region: &'static str,
    pub country: &'static str,
    pub continent: &'static str,
}

impl CityEntry {
    pub const fn lat_deg(self) -> f64 {
        self.lat_e4 as f64 / 10_000.0
    }

    pub const fn lon_deg(self) -> f64 {
        self.lon_e4 as f64 / 10_000.0
    }
}

/// Offline reverse-geocoded place result.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedPlace {
    pub city: String,
    pub region: String,
    pub country: String,
    pub continent: String,
    pub distance_km: f32,
    /// Human-readable display label, e.g. `"Paris, France"` or `"12.34°S, 45.67°E"`.
    pub display: String,
}

/// Curated global gazetteer spanning Europe, Asia, North America, South America, Africa, and Oceania.
pub static WORLD_CITIES: &[CityEntry] = &[
    // Europe
    CityEntry { lat_e4: 488566, lon_e4: 23522, city: "Paris", region: "Ile-de-France", country: "France", continent: "Europe" },
    CityEntry { lat_e4: 437102, lon_e4: 72620, city: "Nice", region: "Provence-Alpes-Cote d'Azur", country: "France", continent: "Europe" },
    CityEntry { lat_e4: 457640, lon_e4: 48357, city: "Lyon", region: "Auvergne-Rhone-Alpes", country: "France", continent: "Europe" },
    CityEntry { lat_e4: 432965, lon_e4: 53698, city: "Marseille", region: "Provence", country: "France", continent: "Europe" },
    CityEntry { lat_e4: 515074, lon_e4: -1278, city: "London", region: "England", country: "UK", continent: "Europe" },
    CityEntry { lat_e4: 559533, lon_e4: -31883, city: "Edinburgh", region: "Scotland", country: "UK", continent: "Europe" },
    CityEntry { lat_e4: 534808, lon_e4: -22426, city: "Manchester", region: "England", country: "UK", continent: "Europe" },
    CityEntry { lat_e4: 533498, lon_e4: -62603, city: "Dublin", region: "Leinster", country: "Ireland", continent: "Europe" },
    CityEntry { lat_e4: 641466, lon_e4: -219426, city: "Reykjavik", region: "Capital Region", country: "Iceland", continent: "Europe" },
    CityEntry { lat_e4: 599139, lon_e4: 107522, city: "Oslo", region: "Eastern Norway", country: "Norway", continent: "Europe" },
    CityEntry { lat_e4: 603913, lon_e4: 53221, city: "Bergen", region: "Vestland", country: "Norway", continent: "Europe" },
    CityEntry { lat_e4: 696492, lon_e4: 189553, city: "Tromso", region: "Troms", country: "Norway", continent: "Europe" },
    CityEntry { lat_e4: 593293, lon_e4: 180686, city: "Stockholm", region: "Uppland", country: "Sweden", continent: "Europe" },
    CityEntry { lat_e4: 556761, lon_e4: 125683, city: "Copenhagen", region: "Hovedstaden", country: "Denmark", continent: "Europe" },
    CityEntry { lat_e4: 601699, lon_e4: 249384, city: "Helsinki", region: "Uusimaa", country: "Finland", continent: "Europe" },
    CityEntry { lat_e4: 523676, lon_e4: 49041, city: "Amsterdam", region: "North Holland", country: "Netherlands", continent: "Europe" },
    CityEntry { lat_e4: 508503, lon_e4: 43517, city: "Brussels", region: "Brussels", country: "Belgium", continent: "Europe" },
    CityEntry { lat_e4: 473769, lon_e4: 85417, city: "Zurich", region: "Zurich", country: "Switzerland", continent: "Europe" },
    CityEntry { lat_e4: 462044, lon_e4: 61432, city: "Geneva", region: "Romandy", country: "Switzerland", continent: "Europe" },
    CityEntry { lat_e4: 525200, lon_e4: 134050, city: "Berlin", region: "Berlin", country: "Germany", continent: "Europe" },
    CityEntry { lat_e4: 481351, lon_e4: 115820, city: "Munich", region: "Bavaria", country: "Germany", continent: "Europe" },
    CityEntry { lat_e4: 535511, lon_e4: 99937, city: "Hamburg", region: "Hamburg", country: "Germany", continent: "Europe" },
    CityEntry { lat_e4: 501109, lon_e4: 86821, city: "Frankfurt", region: "Hesse", country: "Germany", continent: "Europe" },
    CityEntry { lat_e4: 482082, lon_e4: 163738, city: "Vienna", region: "Vienna", country: "Austria", continent: "Europe" },
    CityEntry { lat_e4: 478095, lon_e4: 130550, city: "Salzburg", region: "Salzburg", country: "Austria", continent: "Europe" },
    CityEntry { lat_e4: 500755, lon_e4: 144378, city: "Prague", region: "Bohemia", country: "Czechia", continent: "Europe" },
    CityEntry { lat_e4: 522297, lon_e4: 210122, city: "Warsaw", region: "Masovia", country: "Poland", continent: "Europe" },
    CityEntry { lat_e4: 500647, lon_e4: 199450, city: "Krakow", region: "Lesser Poland", country: "Poland", continent: "Europe" },
    CityEntry { lat_e4: 474979, lon_e4: 190402, city: "Budapest", region: "Central Hungary", country: "Hungary", continent: "Europe" },
    CityEntry { lat_e4: 419028, lon_e4: 124964, city: "Rome", region: "Lazio", country: "Italy", continent: "Europe" },
    CityEntry { lat_e4: 454642, lon_e4: 91900, city: "Milan", region: "Lombardy", country: "Italy", continent: "Europe" },
    CityEntry { lat_e4: 454408, lon_e4: 123155, city: "Venice", region: "Veneto", country: "Italy", continent: "Europe" },
    CityEntry { lat_e4: 437696, lon_e4: 112558, city: "Florence", region: "Tuscany", country: "Italy", continent: "Europe" },
    CityEntry { lat_e4: 408518, lon_e4: 142681, city: "Naples", region: "Campania", country: "Italy", continent: "Europe" },
    CityEntry { lat_e4: 404168, lon_e4: -37038, city: "Madrid", region: "Madrid", country: "Spain", continent: "Europe" },
    CityEntry { lat_e4: 413874, lon_e4: 21686, city: "Barcelona", region: "Catalonia", country: "Spain", continent: "Europe" },
    CityEntry { lat_e4: 373891, lon_e4: -59845, city: "Seville", region: "Andalusia", country: "Spain", continent: "Europe" },
    CityEntry { lat_e4: 387223, lon_e4: -91393, city: "Lisbon", region: "Estremadura", country: "Portugal", continent: "Europe" },
    CityEntry { lat_e4: 411579, lon_e4: -86291, city: "Porto", region: "Norte", country: "Portugal", continent: "Europe" },
    CityEntry { lat_e4: 379838, lon_e4: 237275, city: "Athens", region: "Attica", country: "Greece", continent: "Europe" },
    CityEntry { lat_e4: 363932, lon_e4: 254615, city: "Santorini", region: "Cyclades", country: "Greece", continent: "Europe" },
    CityEntry { lat_e4: 410082, lon_e4: 289784, city: "Istanbul", region: "Marmara", country: "Turkey", continent: "Europe" },
    CityEntry { lat_e4: 444268, lon_e4: 261025, city: "Bucharest", region: "Muntenia", country: "Romania", continent: "Europe" },
    CityEntry { lat_e4: 594370, lon_e4: 247536, city: "Tallinn", region: "Harju", country: "Estonia", continent: "Europe" },

    // Asia & Middle East
    CityEntry { lat_e4: 356762, lon_e4: 1396503, city: "Tokyo", region: "Kanto", country: "Japan", continent: "Asia" },
    CityEntry { lat_e4: 350116, lon_e4: 1357681, city: "Kyoto", region: "Kansai", country: "Japan", continent: "Asia" },
    CityEntry { lat_e4: 346937, lon_e4: 1355023, city: "Osaka", region: "Kansai", country: "Japan", continent: "Asia" },
    CityEntry { lat_e4: 430618, lon_e4: 1413545, city: "Sapporo", region: "Hokkaido", country: "Japan", continent: "Asia" },
    CityEntry { lat_e4: 343853, lon_e4: 1324553, city: "Hiroshima", region: "Chugoku", country: "Japan", continent: "Asia" },
    CityEntry { lat_e4: 375665, lon_e4: 1269780, city: "Seoul", region: "Gyeonggi", country: "South Korea", continent: "Asia" },
    CityEntry { lat_e4: 351796, lon_e4: 1290756, city: "Busan", region: "Yeongnam", country: "South Korea", continent: "Asia" },
    CityEntry { lat_e4: 250330, lon_e4: 1215654, city: "Taipei", region: "Northern Taiwan", country: "Taiwan", continent: "Asia" },
    CityEntry { lat_e4: 223193, lon_e4: 1141694, city: "Hong Kong", region: "Hong Kong", country: "China", continent: "Asia" },
    CityEntry { lat_e4: 399042, lon_e4: 1164074, city: "Beijing", region: "Beijing", country: "China", continent: "Asia" },
    CityEntry { lat_e4: 312304, lon_e4: 1214737, city: "Shanghai", region: "East China", country: "China", continent: "Asia" },
    CityEntry { lat_e4: 225431, lon_e4: 1140579, city: "Shenzhen", region: "Guangdong", country: "China", continent: "Asia" },
    CityEntry { lat_e4: 305728, lon_e4: 1040668, city: "Chengdu", region: "Sichuan", country: "China", continent: "Asia" },
    CityEntry { lat_e4: 13521, lon_e4: 1038198, city: "Singapore", region: "Central", country: "Singapore", continent: "Asia" },
    CityEntry { lat_e4: 137563, lon_e4: 1005018, city: "Bangkok", region: "Central Thailand", country: "Thailand", continent: "Asia" },
    CityEntry { lat_e4: 187883, lon_e4: 989853, city: "Chiang Mai", region: "Northern Thailand", country: "Thailand", continent: "Asia" },
    CityEntry { lat_e4: 78804, lon_e4: 983923, city: "Phuket", region: "Southern Thailand", country: "Thailand", continent: "Asia" },
    CityEntry { lat_e4: 31390, lon_e4: 1016869, city: "Kuala Lumpur", region: "Selangor", country: "Malaysia", continent: "Asia" },
    CityEntry { lat_e4: -62088, lon_e4: 1068456, city: "Jakarta", region: "Java", country: "Indonesia", continent: "Asia" },
    CityEntry { lat_e4: -84095, lon_e4: 1151889, city: "Bali", region: "Lesser Sunda", country: "Indonesia", continent: "Asia" },
    CityEntry { lat_e4: 145995, lon_e4: 1209842, city: "Manila", region: "Luzon", country: "Philippines", continent: "Asia" },
    CityEntry { lat_e4: 210278, lon_e4: 1058342, city: "Hanoi", region: "Red River Delta", country: "Vietnam", continent: "Asia" },
    CityEntry { lat_e4: 108231, lon_e4: 1066297, city: "Ho Chi Minh City", region: "Southeast Vietnam", country: "Vietnam", continent: "Asia" },
    CityEntry { lat_e4: 190760, lon_e4: 728777, city: "Mumbai", region: "Maharashtra", country: "India", continent: "Asia" },
    CityEntry { lat_e4: 286139, lon_e4: 772090, city: "New Delhi", region: "Delhi", country: "India", continent: "Asia" },
    CityEntry { lat_e4: 129716, lon_e4: 775946, city: "Bengaluru", region: "Karnataka", country: "India", continent: "Asia" },
    CityEntry { lat_e4: 277172, lon_e4: 853240, city: "Kathmandu", region: "Bagmati", country: "Nepal", continent: "Asia" },
    CityEntry { lat_e4: 252048, lon_e4: 552708, city: "Dubai", region: "Dubai", country: "UAE", continent: "Asia" },
    CityEntry { lat_e4: 244539, lon_e4: 543773, city: "Abu Dhabi", region: "Abu Dhabi", country: "UAE", continent: "Asia" },
    CityEntry { lat_e4: 252854, lon_e4: 515310, city: "Doha", region: "Ad-Dawhah", country: "Qatar", continent: "Asia" },
    CityEntry { lat_e4: 320853, lon_e4: 347818, city: "Tel Aviv", region: "Center", country: "Israel", continent: "Asia" },

    // North America
    CityEntry { lat_e4: 407128, lon_e4: -740060, city: "New York", region: "New York", country: "USA", continent: "North America" },
    CityEntry { lat_e4: 423601, lon_e4: -710589, city: "Boston", region: "Massachusetts", country: "USA", continent: "North America" },
    CityEntry { lat_e4: 389072, lon_e4: -770369, city: "Washington DC", region: "District of Columbia", country: "USA", continent: "North America" },
    CityEntry { lat_e4: 257617, lon_e4: -801918, city: "Miami", region: "Florida", country: "USA", continent: "North America" },
    CityEntry { lat_e4: 418781, lon_e4: -876298, city: "Chicago", region: "Illinois", country: "USA", continent: "North America" },
    CityEntry { lat_e4: 302672, lon_e4: -977431, city: "Austin", region: "Texas", country: "USA", continent: "North America" },
    CityEntry { lat_e4: 397392, lon_e4: -1049903, city: "Denver", region: "Colorado", country: "USA", continent: "North America" },
    CityEntry { lat_e4: 334484, lon_e4: -1120740, city: "Phoenix", region: "Arizona", country: "USA", continent: "North America" },
    CityEntry { lat_e4: 361699, lon_e4: -1151398, city: "Las Vegas", region: "Nevada", country: "USA", continent: "North America" },
    CityEntry { lat_e4: 340522, lon_e4: -1182437, city: "Los Angeles", region: "California", country: "USA", continent: "North America" },
    CityEntry { lat_e4: 327157, lon_e4: -1171611, city: "San Diego", region: "California", country: "USA", continent: "North America" },
    CityEntry { lat_e4: 377749, lon_e4: -1224194, city: "San Francisco", region: "California", country: "USA", continent: "North America" },
    CityEntry { lat_e4: 455152, lon_e4: -1226784, city: "Portland", region: "Oregon", country: "USA", continent: "North America" },
    CityEntry { lat_e4: 476062, lon_e4: -1223321, city: "Seattle", region: "Washington", country: "USA", continent: "North America" },
    CityEntry { lat_e4: 213069, lon_e4: -1578583, city: "Honolulu", region: "Hawaii", country: "USA", continent: "North America" },
    CityEntry { lat_e4: 612181, lon_e4: -1499003, city: "Anchorage", region: "Alaska", country: "USA", continent: "North America" },
    CityEntry { lat_e4: 436532, lon_e4: -793832, city: "Toronto", region: "Ontario", country: "Canada", continent: "North America" },
    CityEntry { lat_e4: 455017, lon_e4: -735673, city: "Montreal", region: "Quebec", country: "Canada", continent: "North America" },
    CityEntry { lat_e4: 492827, lon_e4: -1231207, city: "Vancouver", region: "British Columbia", country: "Canada", continent: "North America" },
    CityEntry { lat_e4: 511784, lon_e4: -1155708, city: "Banff", region: "Alberta", country: "Canada", continent: "North America" },
    CityEntry { lat_e4: 194326, lon_e4: -991332, city: "Mexico City", region: "CDMX", country: "Mexico", continent: "North America" },
    CityEntry { lat_e4: 211619, lon_e4: -868515, city: "Cancun", region: "Quintana Roo", country: "Mexico", continent: "North America" },
    CityEntry { lat_e4: 231136, lon_e4: -823666, city: "Havana", region: "La Habana", country: "Cuba", continent: "North America" },

    // South America
    CityEntry { lat_e4: -235505, lon_e4: -466333, city: "Sao Paulo", region: "Sao Paulo", country: "Brazil", continent: "South America" },
    CityEntry { lat_e4: -229068, lon_e4: -431729, city: "Rio de Janeiro", region: "Rio de Janeiro", country: "Brazil", continent: "South America" },
    CityEntry { lat_e4: -346037, lon_e4: -583816, city: "Buenos Aires", region: "Buenos Aires", country: "Argentina", continent: "South America" },
    CityEntry { lat_e4: -334489, lon_e4: -706693, city: "Santiago", region: "Metropolitana", country: "Chile", continent: "South America" },
    CityEntry { lat_e4: -120464, lon_e4: -770428, city: "Lima", region: "Lima", country: "Peru", continent: "South America" },
    CityEntry { lat_e4: -135320, lon_e4: -719675, city: "Cusco", region: "Cusco", country: "Peru", continent: "South America" },
    CityEntry { lat_e4: 47110, lon_e4: -740721, city: "Bogota", region: "Cundinamarca", country: "Colombia", continent: "South America" },

    // Africa
    CityEntry { lat_e4: 300444, lon_e4: 312357, city: "Cairo", region: "Lower Egypt", country: "Egypt", continent: "Africa" },
    CityEntry { lat_e4: 316295, lon_e4: -79811, city: "Marrakech", region: "Marrakesh-Safi", country: "Morocco", continent: "Africa" },
    CityEntry { lat_e4: 65244, lon_e4: 33792, city: "Lagos", region: "Lagos", country: "Nigeria", continent: "Africa" },
    CityEntry { lat_e4: -12921, lon_e4: 368219, city: "Nairobi", region: "Nairobi", country: "Kenya", continent: "Africa" },
    CityEntry { lat_e4: -262041, lon_e4: 280473, city: "Johannesburg", region: "Gauteng", country: "South Africa", continent: "Africa" },
    CityEntry { lat_e4: -339249, lon_e4: 184241, city: "Cape Town", region: "Western Cape", country: "South Africa", continent: "Africa" },

    // Oceania
    CityEntry { lat_e4: -338688, lon_e4: 1512093, city: "Sydney", region: "New South Wales", country: "Australia", continent: "Oceania" },
    CityEntry { lat_e4: -378136, lon_e4: 1449631, city: "Melbourne", region: "Victoria", country: "Australia", continent: "Oceania" },
    CityEntry { lat_e4: -274698, lon_e4: 1530251, city: "Brisbane", region: "Queensland", country: "Australia", continent: "Oceania" },
    CityEntry { lat_e4: -319505, lon_e4: 1158605, city: "Perth", region: "Western Australia", country: "Australia", continent: "Oceania" },
    CityEntry { lat_e4: -368485, lon_e4: 1747633, city: "Auckland", region: "North Island", country: "New Zealand", continent: "Oceania" },
    CityEntry { lat_e4: -450312, lon_e4: 1686626, city: "Queenstown", region: "Otago", country: "New Zealand", continent: "Oceania" },
];

/// Great-circle distance in kilometers between two WGS84 coordinates using the Haversine formula.
pub fn haversine_km(lat1_deg: f64, lon1_deg: f64, lat2_deg: f64, lon2_deg: f64) -> f64 {
    const EARTH_RADIUS_KM: f64 = 6371.0088;
    let to_rad = core::f64::consts::PI / 180.0;
    let dlat = (lat2_deg - lat1_deg) * to_rad;
    let dlon = (lon2_deg - lon1_deg) * to_rad;
    let lat1 = lat1_deg * to_rad;
    let lat2 = lat2_deg * to_rad;

    let sin_dlat = libm::sin(dlat * 0.5);
    let sin_dlon = libm::sin(dlon * 0.5);
    let a = sin_dlat * sin_dlat + libm::cos(lat1) * libm::cos(lat2) * sin_dlon * sin_dlon;
    let c = 2.0 * libm::asin(libm::sqrt(a.clamp(0.0, 1.0)));
    EARTH_RADIUS_KM * c
}

/// Resolves decimal degrees `(lat, lon)` offline against [`WORLD_CITIES`].
///
/// Returns `None` if `lat` or `lon` is NaN, infinite, or outside `[-90..=90, -180..=180]`.
/// When within [`MAX_CITY_MATCH_KM`] of a known city, returns `"City, Country"`; otherwise
/// formats a hemisphere coordinate bucket (e.g. `"48.86°N, 2.35°E"`).
pub fn resolve_place(lat: f64, lon: f64) -> Option<ResolvedPlace> {
    if !lat.is_finite() || !lon.is_finite() || !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }

    let mut best: Option<(&CityEntry, f64)> = None;
    for entry in WORLD_CITIES {
        let dist = haversine_km(lat, lon, entry.lat_deg(), entry.lon_deg());
        if best.map_or(true, |(_, b)| dist < b) {
            best = Some((entry, dist));
        }
    }

    if let Some((entry, dist_km)) = best {
        if dist_km <= MAX_CITY_MATCH_KM {
            return Some(ResolvedPlace {
                city: entry.city.to_string(),
                region: entry.region.to_string(),
                country: entry.country.to_string(),
                continent: entry.continent.to_string(),
                distance_km: dist_km as f32,
                display: format!("{}, {}", entry.city, entry.country),
            });
        }
    }

    let ns = if lat >= 0.0 { 'N' } else { 'S' };
    let ew = if lon >= 0.0 { 'E' } else { 'W' };
    let display = format!("{:.2}°{}, {:.2}°{}", libm::fabs(lat), ns, libm::fabs(lon), ew);
    Some(ResolvedPlace {
        city: String::new(),
        region: "Remote".to_string(),
        country: String::new(),
        continent: "Coordinates".to_string(),
        distance_km: best.map(|(_, d)| d as f32).unwrap_or(0.0),
        display,
    })
}
