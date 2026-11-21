use super::{blend, blend_solid_span};
use solor::Rgba;

#[test]
fn solid_spans_match_individual_pixels_for_every_alpha_and_coverage() {
    let background = [
        [0, 255, 127, 255],
        [255, 0, 128, 255],
        [33, 151, 219, 255],
        [0; 4],
        [10, 80, 48, 97],
    ];
    for alpha in 0..=255 {
        let color = Rgba {
            red: 255,
            green: 0,
            blue: 73,
            alpha,
        };
        for coverage in 0..=255 {
            let mut expected = background;
            for pixel in &mut expected {
                blend(pixel, color, coverage);
            }
            let mut actual = background;
            blend_solid_span(&mut actual, color, coverage);
            assert_eq!(actual, expected, "alpha={alpha}, coverage={coverage}");
        }
    }
}
