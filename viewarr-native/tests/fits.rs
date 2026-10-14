//! Cross-checks of the FITS reader against fixtures written by astropy.
//!
//! The expected values here were produced by reading the same files with
//! `astropy.io.fits`, so these tests pin our reader to an independent
//! implementation rather than to its own assumptions. Each fixture encodes its
//! own coordinates in its pixel values (`1000*z + 10*y + x` and friends), so a
//! transposed, flipped, or mis-strided read fails loudly instead of subtly.

use std::path::{Path, PathBuf};

use viewarr_native::fits::{FitsFile, HduKind, Value};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

fn open(name: &str) -> FitsFile {
    FitsFile::open(&fixture(name)).expect("fixture should open")
}

/// Compare ignoring NaN-vs-NaN inequality, which `assert_eq!` would trip over.
fn assert_pixels(actual: &[f64], expected: &[f64]) {
    assert_eq!(actual.len(), expected.len(), "pixel count");
    for (i, (a, e)) in actual.iter().zip(expected).enumerate() {
        if e.is_nan() {
            assert!(a.is_nan(), "pixel {i}: expected NaN, got {a}");
        } else {
            assert!((a - e).abs() < 1e-9, "pixel {i}: expected {e}, got {a}");
        }
    }
}

#[test]
fn reads_a_2d_float_image_in_fits_order() {
    let fits = open("image2d.fits");
    assert_eq!(fits.hdus.len(), 1);
    assert_eq!(fits.default_image_hdu(), Some(0));

    let hdu = &fits.hdus[0];
    assert_eq!(hdu.name, "PRIMARY");
    assert_eq!(hdu.axes, vec![6, 4]);
    assert_eq!((hdu.width(), hdu.height()), (6, 4));
    assert!(hdu.slice_dims().is_empty(), "a 2-D image has no slice axes");
    assert!(!hdu.is_integer());
    assert_eq!(hdu.value_decimals(), 4);

    // value = 10*y + x, with the last pixel blanked to NaN. Row 0 comes first,
    // which is the bottom row in FITS convention — exactly the order the widget
    // wants, so no flip happens anywhere.
    let expected: Vec<f64> = vec![
        0.0,
        1.0,
        2.0,
        3.0,
        4.0,
        5.0, //
        10.0,
        11.0,
        12.0,
        13.0,
        14.0,
        15.0, //
        20.0,
        21.0,
        22.0,
        23.0,
        24.0,
        25.0, //
        30.0,
        31.0,
        32.0,
        33.0,
        34.0,
        f64::NAN,
    ];
    assert_pixels(&fits.read_plane(hdu, &[]).unwrap(), &expected);
}

#[test]
fn reads_unsigned_16_bit_cube_planes() {
    let fits = open("cube3d.fits");
    let hdu = &fits.hdus[0];

    assert_eq!(hdu.axes, vec![5, 3, 4]);
    assert_eq!(
        hdu.slice_dims(),
        vec![4],
        "NAXIS3 is the one sliceable axis"
    );
    // BITPIX 16 with BZERO 32768 is the standard unsigned-16 encoding; it still
    // yields whole numbers, so the readout should stay integral.
    assert!(hdu.is_integer());
    assert_eq!(hdu.value_decimals(), 0);
    assert_eq!(hdu.shape_string(), "5x3x4");

    let plane0: Vec<f64> = vec![
        0.0, 1.0, 2.0, 3.0, 4.0, 10.0, 11.0, 12.0, 13.0, 14.0, 20.0, 21.0, 22.0, 23.0, 24.0,
    ];
    assert_pixels(&fits.read_plane(hdu, &[0]).unwrap(), &plane0);

    let plane3: Vec<f64> = vec![
        3000.0, 3001.0, 3002.0, 3003.0, 3004.0, 3010.0, 3011.0, 3012.0, 3013.0, 3014.0, 3020.0,
        3021.0, 3022.0, 3023.0, 3024.0,
    ];
    assert_pixels(&fits.read_plane(hdu, &[3]).unwrap(), &plane3);

    assert!(
        fits.read_plane(hdu, &[4]).is_err(),
        "an out-of-range index should be refused, not read out of bounds"
    );
    assert!(
        fits.read_plane(hdu, &[]).is_err(),
        "the wrong number of indices should be refused"
    );
}

#[test]
fn orders_4d_slice_axes_outer_to_inner() {
    let fits = open("cube4d.fits");
    let hdu = &fits.hdus[0];

    // On disk: NAXIS1=4, NAXIS2=2, NAXIS3=2, NAXIS4=3.
    assert_eq!(hdu.axes, vec![4, 2, 2, 3]);
    // The widget wants outer-to-inner, i.e. [NAXIS4, NAXIS3] — the reverse.
    assert_eq!(hdu.slice_dims(), vec![3, 2]);

    // value = 1000*t + 100*z + 10*y + x, so these indices can only match if the
    // axis order and the plane stride are both right.
    let t2z1: Vec<f64> = vec![
        2100.0, 2101.0, 2102.0, 2103.0, 2110.0, 2111.0, 2112.0, 2113.0,
    ];
    assert_pixels(&fits.read_plane(hdu, &[2, 1]).unwrap(), &t2z1);

    let t0z1: Vec<f64> = vec![100.0, 101.0, 102.0, 103.0, 110.0, 111.0, 112.0, 113.0];
    assert_pixels(&fits.read_plane(hdu, &[0, 1]).unwrap(), &t0z1);
}

#[test]
fn skips_a_dataless_primary_and_finds_named_extensions() {
    let fits = open("mef.fits");
    assert_eq!(fits.hdus.len(), 3);

    assert!(
        !fits.hdus[0].is_viewable_image(),
        "the primary HDU carries no data"
    );
    assert_eq!(fits.hdus[1].name, "SCI");
    assert_eq!(fits.hdus[2].name, "ERR");
    assert_eq!(
        fits.default_image_hdu(),
        Some(1),
        "the first HDU with an image should be chosen"
    );

    let sci = &fits.hdus[1];
    assert_eq!(sci.axes, vec![4, 3]);
    assert_pixels(
        &fits.read_plane(sci, &[]).unwrap(),
        &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0],
    );

    // Reaching the second extension proves the data unit of the first was
    // stepped over by exactly the right number of 2880-byte blocks.
    let err = &fits.hdus[2];
    assert_pixels(&fits.read_plane(err, &[]).unwrap(), &[2.5; 12]);
}

#[test]
fn applies_a_non_unit_bscale_as_floating_point() {
    let fits = open("scaled.fits");
    let hdu = &fits.hdus[0];

    // Stored as int16 with BSCALE 0.25 / BZERO 100, so the physical values are
    // not whole numbers in general and must not be shown as integers.
    assert!(!hdu.is_integer());
    assert_eq!(hdu.value_decimals(), 6);

    let expected: Vec<f64> = vec![
        0.0, 1.0, 2.0, 3.0, 4.0, 5.0, //
        10.0, 11.0, 12.0, 13.0, 14.0, 15.0, //
        20.0, 21.0, 22.0, 23.0, 24.0, 25.0, //
        30.0, 31.0, 32.0, 33.0, 34.0, 0.0,
    ];
    assert_pixels(&fits.read_plane(hdu, &[]).unwrap(), &expected);
}

#[test]
fn reports_tile_compressed_images_instead_of_misreading_them() {
    let fits = open("compressed.fits");
    let hdu = &fits.hdus[1];

    assert!(
        hdu.compressed,
        "a BINTABLE with ZIMAGE=T is a compressed image"
    );
    assert!(!hdu.is_viewable_image());
    // The real image shape comes from ZNAXISn, so the error message can say
    // what was skipped rather than describing the compressed-tile table.
    assert_eq!(hdu.axes, vec![6, 4]);
    assert_eq!(
        fits.default_image_hdu(),
        None,
        "nothing in this file is readable yet"
    );
}

#[test]
fn rejects_files_that_are_not_fits() {
    let path = std::env::temp_dir().join("viewarr-not-a-fits-file.txt");
    std::fs::write(&path, vec![b'x'; 4000]).unwrap();
    let err = match FitsFile::open(&path) {
        Err(err) => err,
        Ok(_) => panic!("a text file should not open as FITS"),
    };
    assert!(
        format!("{err:#}").contains("does not look like a FITS file"),
        "unexpected message: {err:#}"
    );
    std::fs::remove_file(&path).ok();
}

/// The header viewer shows a header as the file actually has it, so the reader
/// has to keep what a reader interested only in pixels would drop: comments,
/// the valueless COMMENT/HISTORY cards, and the order and duplication of both.
#[test]
fn keeps_whole_header_cards_in_file_order() {
    let fits = open("header.fits");
    let cards = &fits.hdus[0].header.cards;

    let find = |key: &str| cards.iter().find(|c| c.key == key).expect(key);

    // A value and its comment both survive.
    let exptime = find("EXPTIME");
    assert_eq!(exptime.value, Some(Value::Float(1.5)));
    assert_eq!(exptime.comment.as_deref(), Some("seconds"));

    // A string keeps its quotes when displayed, so it cannot be mistaken for
    // a number, and is trimmed of the padding FITS writes.
    assert_eq!(find("OBSERVER").value, Some(Value::Str("J. Long".into())));
    assert_eq!(
        find("OBSERVER").value.as_ref().unwrap().display(),
        "'J. Long'"
    );
    assert_eq!(find("QUALITY").value, Some(Value::Bool(true)));

    // A card with no comment has none invented for it.
    assert_eq!(find("BARE").value, Some(Value::Int(42)));
    assert_eq!(find("BARE").comment, None);

    // COMMENT and HISTORY carry free text and no value.
    let free: Vec<(&str, &str)> = cards
        .iter()
        .filter(|c| c.key == "COMMENT" || c.key == "HISTORY")
        .map(|c| (c.key.as_str(), c.comment.as_deref().unwrap_or("")))
        .collect();
    assert_eq!(
        free,
        vec![
            ("COMMENT", "first comment"),
            ("COMMENT", "second comment"),
            ("HISTORY", "step one"),
            ("HISTORY", "step two"),
        ],
        "duplicate keywords and their order both survive"
    );
    assert!(cards
        .iter()
        .all(|c| c.key != "COMMENT" || c.value.is_none()));

    // Order is the file's, not sorted or grouped.
    let keys: Vec<&str> = cards.iter().map(|c| c.key.as_str()).collect();
    let simple = keys.iter().position(|k| *k == "SIMPLE").unwrap();
    let observer = keys.iter().position(|k| *k == "OBSERVER").unwrap();
    let history = keys.iter().position(|k| *k == "HISTORY").unwrap();
    assert!(simple < observer && observer < history, "{keys:?}");

    // END is a terminator, not a card, and the blank padding after it is not
    // shown as a row of empty keys.
    assert!(!keys.contains(&"END"));
    assert!(
        cards
            .iter()
            .all(|c| !c.key.is_empty() || c.comment.is_some()),
        "no blank padding cards"
    );
}

/// The keywords the reader itself needs still resolve out of the fuller
/// header, which is the part that could silently regress.
#[test]
fn still_reads_the_keywords_it_interprets() {
    let fits = open("header.fits");
    let hdu = &fits.hdus[0];
    assert_eq!(hdu.axes, vec![4, 3]);
    assert_eq!(hdu.bitpix.name(), "i16");
    assert_eq!(hdu.name, "PRIMARY");
    assert_pixels(
        &fits.read_plane(hdu, &[]).unwrap(),
        &(0..12).map(f64::from).collect::<Vec<_>>(),
    );
}

#[test]
fn filtering_a_header_looks_at_keys_values_and_comments() {
    let fits = open("header.fits");
    let cards = &fits.hdus[0].header.cards;
    let matching = |needle: &str| {
        cards
            .iter()
            .filter(|c| c.matches(needle))
            .map(|c| c.key.as_str())
            .collect::<Vec<_>>()
    };

    assert_eq!(
        matching("observer"),
        vec!["OBSERVER"],
        "key, case-insensitive"
    );
    assert_eq!(matching("camsci"), vec!["INSTRUME"], "value");
    assert_eq!(matching("passed qa"), vec!["QUALITY"], "comment");
    assert_eq!(
        matching("step").len(),
        2,
        "free text of valueless cards is searchable too"
    );
    assert_eq!(
        matching("").len(),
        cards.len(),
        "an empty filter hides nothing"
    );
    assert!(matching("nothinghere").is_empty());
}

/// A HIERARCH card carries a multi-word keyword where a standard card has its
/// value indicator. The keyword is what follows the marker -- `HIERARCH`
/// announces the convention rather than naming the card -- which is both what
/// the standard says and what `astropy` reports.
#[test]
fn reads_a_hierarch_keyword_without_its_marker() {
    let fits = open("header.fits");
    let card = fits.hdus[0]
        .header
        .cards
        .iter()
        .find(|c| c.key == "long keyword here")
        .expect("the fixture has a HIERARCH card");

    assert_eq!(card.value, Some(Value::Int(7)));
    assert_eq!(card.comment, None);
    assert!(
        !fits.hdus[0]
            .header
            .cards
            .iter()
            .any(|c| c.key == "HIERARCH"),
        "the marker is not a keyword"
    );

    // Searchable by any of its words, which is the point of keeping the
    // keyword whole rather than stopping at eight characters.
    assert!(card.matches("long keyword"));
    assert!(card.matches("here"));
}

/// Prose with an `=` in it stays prose: only HIERARCH gets the extended hunt
/// for a value indicator.
#[test]
fn an_equals_sign_in_comment_text_is_not_a_value_indicator() {
    let fits = open("eqcomment.fits");
    let cards = &fits.hdus[0].header.cards;
    let comment = cards
        .iter()
        .find(|c| c.key == "COMMENT")
        .expect("the fixture has a COMMENT card");

    assert_eq!(comment.value, None, "a comment has no value, = or not");
    assert_eq!(comment.comment.as_deref(), Some("ratio = 3 to 1"));
}

/// Long values and comments are split across CONTINUE cards by the writer;
/// the reader folds them back into one card. Expected values here are what
/// `astropy.io.fits` reports for the same file, so this pins us to its
/// handling of over-long lines rather than to our own reading of the spec.
#[test]
fn folds_continue_cards_back_into_one_value_and_comment() {
    let fits = open("longcards.fits");
    let cards = &fits.hdus[0].header.cards;
    let card = |key: &str| {
        cards
            .iter()
            .find(|c| c.key == key)
            .unwrap_or_else(|| panic!("no card {key}"))
    };
    let string_of = |key: &str| match &card(key).value {
        Some(Value::Str(text)) => text.clone(),
        other => panic!("{key} should be a string, got {other:?}"),
    };

    // A value too long for one card, whose comment also needed its own.
    assert_eq!(string_of("LONGSTR"), "x".repeat(120));
    assert_eq!(
        card("LONGSTR").comment.as_deref(),
        Some("a comment that is itself fairly long so it may wrap too")
    );

    // A HIERARCH keyword leaves so little room that value and comment both
    // spill -- the case that makes folding worth doing.
    assert_eq!(
        string_of("a very long hierarch keyword indeed"),
        "y".repeat(90)
    );
    assert_eq!(
        card("a very long hierarch keyword indeed")
            .comment
            .as_deref(),
        Some("hierarch comment also long enough to be interesting here")
    );

    // A short HIERARCH loses its marker just the same.
    assert_eq!(string_of("ESO DET CHIP1 NAME"), "chip-a");
    assert_eq!(
        card("ESO DET CHIP1 NAME").comment.as_deref(),
        Some("detector")
    );

    // A string that merely ends in '&' and is never continued keeps it: the
    // marker only means continuation when a CONTINUE actually follows.
    assert_eq!(string_of("AMPER"), "ends with &");
    assert_eq!(string_of("SHORT"), "fits");

    // The folded-away cards are gone, not left as rows of their own.
    assert!(!cards.iter().any(|c| c.key == "CONTINUE"));
    // astropy counts ten cards in this file; so should we. Catches a fold
    // that drops a card or leaves a fragment behind, which the per-key
    // assertions above would not notice.
    assert_eq!(
        cards.len(),
        10,
        "{:?}",
        cards.iter().map(|c| &c.key).collect::<Vec<_>>()
    );
}

/// An orphan CONTINUE has nothing to fold into. astropy rejects the whole
/// file; a viewer is more useful showing the card than refusing to open the
/// image beside it.
#[test]
fn an_orphan_continue_is_shown_rather_than_rejected() {
    let fits = open("orphan.fits");
    let cards = &fits.hdus[0].header.cards;

    let orphan = cards
        .iter()
        .find(|c| c.key == "CONTINUE")
        .expect("the stray card survives as itself");
    assert_eq!(orphan.value, Some(Value::Str("nothing precedes me".into())));

    // The integer card before it is untouched.
    let int_card = cards.iter().find(|c| c.key == "INTVAL").unwrap();
    assert_eq!(int_card.value, Some(Value::Int(42)));
}

/// A table declares NAXIS=2 with NAXIS1 the row length in bytes and NAXIS2
/// the row count, so by shape alone it is indistinguishable from an image —
/// and was being drawn as one. Only XTENSION tells them apart.
#[test]
fn tables_are_not_images_however_much_their_shape_looks_like_one() {
    let fits = open("table.fits");

    let kinds: Vec<(&str, &HduKind)> = fits
        .hdus
        .iter()
        .map(|h| (h.name.as_str(), &h.kind))
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("PRIMARY", &HduKind::Image),
            ("EVENTS", &HduKind::BinTable),
            ("CATALOG", &HduKind::AsciiTable),
            ("SCI", &HduKind::Image),
        ]
    );

    let hdu = |name: &str| fits.hdus.iter().find(|h| h.name == name).unwrap();

    // Both tables have two axes and real data, which is exactly why shape
    // alone let them through before.
    for name in ["EVENTS", "CATALOG"] {
        let table = hdu(name);
        assert_eq!(table.axes.len(), 2, "{name} looks 2-D by shape");
        assert!(!table.is_viewable_image(), "{name} must not be drawable");
    }
    assert_eq!(
        hdu("EVENTS").unviewable_reason(),
        Some("a binary table, not an image")
    );
    assert_eq!(
        hdu("CATALOG").unviewable_reason(),
        Some("an ASCII table, not an image")
    );

    // The shape column would otherwise report bytes-per-row by rows.
    assert_eq!(hdu("EVENTS").shape_label(), "binary table");
    assert_eq!(hdu("CATALOG").shape_label(), "ASCII table");

    // The real images are untouched.
    assert!(hdu("PRIMARY").is_viewable_image());
    assert!(hdu("SCI").is_viewable_image());
    assert_eq!(hdu("SCI").shape_label(), "3x2");
    assert_eq!(hdu("SCI").unviewable_reason(), None);
}

/// Opening the file must land on an image, and asking for a table by name
/// must say what it is rather than claiming there is no data.
#[test]
fn hdu_selection_skips_tables_and_explains_them_when_asked_for() {
    let fits = open("table.fits");

    assert_eq!(
        fits.resolve_hdu(None).unwrap(),
        0,
        "the primary image, not the first thing with two axes"
    );
    assert_eq!(fits.resolve_hdu(Some("SCI")).unwrap(), 3);

    let err = format!("{:#}", fits.resolve_hdu(Some("EVENTS")).unwrap_err());
    assert!(err.contains("binary table"), "{err}");

    let err = format!("{:#}", fits.resolve_hdu(Some("CATALOG")).unwrap_err());
    assert!(err.contains("ASCII table"), "{err}");
}
