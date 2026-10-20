use super::*;

const SELECTED: Atom = 7;
const TARGET: u32 = 1;
const BODY: Size = Size::new(160, 160);

fn property(values: &[u32]) -> GetPropertyReply {
    GetPropertyReply {
        format: 32,
        sequence: 0,
        length: values.len().try_into().unwrap(),
        type_: AtomEnum::CARDINAL.into(),
        bytes_after: 0,
        value_len: values.len().try_into().unwrap(),
        value: values.iter().flat_map(|v| v.to_ne_bytes()).collect(),
    }
}

fn absent() -> GetPropertyReply {
    GetPropertyReply {
        type_: x11rb::NONE,
        format: 0,
        ..property(&[])
    }
}

fn malformed_replies(valid: GetPropertyReply) -> Vec<GetPropertyReply> {
    (0..8)
        .map(|case| {
            let mut p = valid.clone();
            match case {
                0 => p.type_ = AtomEnum::STRING.into(),
                1 => p.format = 16,
                2 => p.bytes_after = 4,
                3 => p.length += 1,
                4 => p.value_len += 1,
                5 => {
                    p.value.pop();
                }
                6 => p.value.push(0),
                _ => p.type_ = x11rb::NONE, // Malformed absence must never allow fallback.
            }
            p
        })
        .collect()
}

fn monitor(atom: Atom, primary: bool) -> MonitorGeometry {
    MonitorGeometry {
        name: format!("monitor-{atom}"),
        name_atom: atom,
        outputs: vec![10],
        is_primary: primary,
        x: -1920,
        y: -100,
        width: 1920,
        height: 1080,
    }
}

fn snapshot() -> DestinationSnapshot {
    DestinationSnapshot {
        count: property(&[2]),
        current: property(&[TARGET]),
        workarea: property(&[
            10,
            20,
            400,
            300,
            (-1900_i32) as u32,
            (-80_i32) as u32,
            1800,
            900,
        ]),
        viewport: property(&[0; 4]),
        monitors: vec![monitor(SELECTED, false), monitor(8, true)],
    }
}

fn validate(s: &DestinationSnapshot) -> Result<DestinationLayout, DestinationError> {
    validate_snapshot(s, SELECTED, TARGET, BODY)
}

fn workarea(s: &mut DestinationSnapshot, area: Rect) {
    s.workarea = property(&[
        10,
        20,
        400,
        300,
        area.x as u32,
        area.y as u32,
        area.width,
        area.height,
    ]);
}

fn pair(
    before: DestinationSnapshot,
    after: DestinationSnapshot,
) -> Result<DestinationLayout, DestinationError> {
    let mut reads = [before, after].into_iter();
    let result = query_with(SELECTED, TARGET, BODY, || {
        Ok(reads.next().expect("No third read or retry"))
    });
    assert!(
        reads.next().is_none(),
        "Both complete reads must be consumed"
    );
    result
}

fn assert_unverifiable(result: Result<DestinationLayout, DestinationError>) {
    assert!(
        matches!(result, Err(DestinationError::Unverifiable(_))),
        "{result:?}"
    );
}

fn caller_pair(
    before: (GetPropertyReply, GetPropertyReply),
    after: (GetPropertyReply, GetPropertyReply),
) -> Result<CallerDesktop, DestinationError> {
    let mut reads = [before, after].into_iter();
    let result = capture_caller_with(|| Ok(reads.next().expect("No third read or retry")));
    assert!(reads.next().is_none(), "Exactly two complete snapshots");
    result
}

#[test]
fn caller_capture_is_exact_two_read_identity_with_no_desktop_count_cap() {
    for (count, current) in [(1, 0), (7, 6), (u32::MAX, u32::MAX - 1)] {
        let pair = (property(&[count]), property(&[current]));
        let captured = caller_pair(pair.clone(), pair).unwrap();
        assert_eq!(captured.desktop_count, count);
        assert_eq!(captured.frozen_destination(), current);
    }
}

#[test]
fn caller_rejects_all_bad_count_or_current_metadata_in_either_complete_snapshot() {
    for bad_count in [true, false] {
        let bad = malformed_replies(property(&[2])).into_iter().chain([
            absent(),
            property(&[]),
            property(&[1, 2]),
        ]);
        for property in bad {
            for bad_first in [true, false] {
                let good = (self::property(&[3]), self::property(&[2]));
                let mut malformed = good.clone();
                if bad_count {
                    malformed.0 = property.clone();
                } else {
                    malformed.1 = property.clone();
                }
                let result = if bad_first {
                    caller_pair(malformed, good)
                } else {
                    caller_pair(good, malformed)
                };
                assert!(matches!(result, Err(DestinationError::Unverifiable(_))));
            }
        }
    }
}

#[test]
fn caller_rejects_zero_count_sticky_and_out_of_range_current_without_fallback() {
    for (count, current) in [(0, 0), (2, u32::MAX), (2, 2), (2, 3)] {
        let pair = (property(&[count]), property(&[current]));
        assert!(matches!(
            caller_pair(pair.clone(), pair),
            Err(DestinationError::Unverifiable(_))
        ));
    }
}

#[test]
fn caller_two_read_disagreement_rejects_without_a_repair_read() {
    for (count, current) in [(4, 2), (3, 1)] {
        assert_eq!(
            caller_pair(
                (property(&[3]), property(&[2])),
                (property(&[count]), property(&[current]))
            ),
            Err(DestinationError::Changed)
        );
    }
}

#[test]
fn caller_transport_failure_stops_immediately_and_never_retries() {
    for fail_on in [1, 2] {
        let mut reads = 0;
        let result = capture_caller_with(|| {
            reads += 1;
            if reads == fail_on {
                Err(unverifiable("read failed"))
            } else {
                Ok((property(&[3]), property(&[2])))
            }
        });
        assert_eq!(result, Err(unverifiable("read failed")));
        assert_eq!(reads, fail_on);
    }
}

#[test]
fn caller_pre_send_revalidation_requires_the_identical_frozen_count_and_current() {
    let mut snapshots = 0;
    let captured = capture_caller_with(|| {
        snapshots += 1;
        Ok((property(&[3]), property(&[2])))
    })
    .unwrap();
    for (count, current, valid) in [(3, 2, true), (4, 2, false), (3, 1, false)] {
        let before = snapshots;
        let fresh = capture_caller_with(|| {
            snapshots += 1;
            Ok((property(&[count]), property(&[current])))
        })
        .unwrap();
        assert_eq!(snapshots - before, 2);
        assert_eq!(captured.revalidate(fresh).is_ok(), valid);
        assert_eq!(captured.desktop_count, 3);
        assert_eq!(captured.frozen_destination(), 2);
    }
}

#[test]
fn projection_preserves_every_equality_field_workarea_and_viewport_presence() {
    for workarea_present in [true, false] {
        for viewport_present in [true, false] {
            let mut raw = snapshot();
            let retained_atom = 0xf123_4567;
            raw.monitors[0].name_atom = retained_atom;
            if !workarea_present {
                raw.workarea = absent();
            }
            if !viewport_present {
                raw.viewport = absent();
            }
            let layout = validate_snapshot(&raw, retained_atom, TARGET, BODY).unwrap();
            let identity = DestinationIdentity::from(&layout);
            assert_eq!(identity.desktop_count, 2);
            assert_eq!(identity.destination_desktop, TARGET);
            assert_eq!(identity.current_desktop, TARGET);
            assert_eq!(identity.monitor_name, retained_atom);
            assert_eq!(identity.monitor_bounds, Rect::new(-1920, -100, 1920, 1080));
            assert_eq!(
                identity.target_workarea,
                workarea_present.then_some(Rect::new(-1900, -80, 1800, 900))
            );
            assert_eq!(
                identity.viewport,
                if viewport_present {
                    DestinationViewport::AllZero
                } else {
                    DestinationViewport::Absent
                }
            );
            assert_eq!(
                identity.usable_area,
                if workarea_present {
                    Rect::new(-1900, -80, 1800, 900)
                } else {
                    identity.monitor_bounds
                }
            );
            assert_eq!(identity.body_size, BODY);
            assert_eq!(
                identity.valid_origins,
                if workarea_present {
                    ValidOriginBounds::new(-1900, -260, -80, 660)
                } else {
                    ValidOriginBounds::new(-1920, -160, -100, 820)
                }
            );
        }
    }
}

#[test]
fn projection_preserves_representable_signed_boundary_bounds_and_fixed_size() {
    for origin in [i16::MIN, i16::MAX - 160] {
        let mut raw = snapshot();
        raw.monitors[0].x = origin;
        raw.monitors[0].y = origin;
        raw.monitors[0].width = 160;
        raw.monitors[0].height = 160;
        workarea(&mut raw, Rect::new(origin.into(), origin.into(), 160, 160));
        let layout = validate(&raw).unwrap();
        let projected = DestinationIdentity::from(&layout);
        assert_eq!(projected.monitor_bounds.x, i32::from(origin));
        assert_eq!(projected.target_workarea, Some(projected.monitor_bounds));
        assert_eq!(projected.body_size, BODY);
        assert_eq!(
            projected.valid_origins,
            ValidOriginBounds::new(origin.into(), origin.into(), origin.into(), origin.into())
        );
    }
}

#[test]
fn count_is_exactly_one_positive_cardinal_without_an_arbitrary_maximum() {
    for count in [1, 2, 100_000, u32::MAX] {
        assert_eq!(parse_count(&property(&[count])), Ok(count));
    }
}

#[test]
fn absent_zero_empty_and_multiple_counts_are_rejected() {
    for p in [absent(), property(&[0]), property(&[]), property(&[1, 2])] {
        assert!(matches!(
            parse_count(&p),
            Err(DestinationError::Unverifiable(_))
        ));
    }
}

#[test]
fn count_type_format_truncation_and_length_metadata_are_strict() {
    for p in malformed_replies(property(&[2])) {
        assert!(matches!(
            parse_count(&p),
            Err(DestinationError::Unverifiable(_))
        ));
    }
}

#[test]
fn count_minus_one_is_a_valid_frozen_target() {
    let layout = validate(&snapshot()).unwrap();
    assert_eq!(layout.destination_desktop, layout.desktop_count - 1);
}

#[test]
fn target_at_count_beyond_count_and_all_desktops_are_invalid() {
    for target in [2, 3, u32::MAX] {
        assert_unverifiable(validate_snapshot(&snapshot(), SELECTED, target, BODY));
    }
}

#[test]
fn count_multiplication_checks_word_and_host_byte_overflow() {
    assert_eq!(array_words(2, 4), Ok(8));
    assert_eq!(array_words(2, 2), Ok(4));
    for (count, width) in [(u32::MAX, 4), (u32::MAX / 4 + 1, 4), (u32::MAX, 2)] {
        assert!(matches!(
            array_words(count, width),
            Err(DestinationError::Unverifiable(_))
        ));
    }
    // The boundary is representability, not a user-facing desktop-count policy.
    if usize::BITS > 32 {
        assert_eq!(array_words(u32::MAX / 4, 4), Ok(u32::MAX - 3));
    } else {
        assert!(array_words(u32::MAX / 4, 4).is_err());
    }
    let mut s = snapshot();
    s.count = property(&[u32::MAX]);
    s.workarea = absent();
    s.viewport = absent();
    assert_unverifiable(validate(&s));
}

#[test]
fn current_must_equal_frozen_target_and_is_never_retargeted() {
    let mut s = snapshot();
    assert_eq!(validate(&s).unwrap().current_desktop, TARGET);
    s.current = property(&[0]);
    assert_eq!(validate(&s), Err(DestinationError::Changed));
    assert_eq!(one_word(&s.current), Ok(0));
}

#[test]
fn current_at_or_above_count_and_all_desktops_are_unverifiable() {
    for current in [2, 3, u32::MAX] {
        let mut s = snapshot();
        s.current = property(&[current]);
        assert_unverifiable(validate(&s));
    }
}

#[test]
fn current_absence_empty_multiple_and_malformed_replies_are_unverifiable() {
    for p in [absent(), property(&[]), property(&[1, 1])]
        .into_iter()
        .chain(malformed_replies(property(&[1])))
    {
        let mut s = snapshot();
        s.current = p;
        assert_unverifiable(validate(&s));
    }
}

#[test]
fn exact_workarea_array_selects_nonzero_target_and_preserves_signed_origins() {
    let layout = validate(&snapshot()).unwrap();
    assert_eq!(
        layout.target_workarea,
        Some(Rect::new(-1900, -80, 1800, 900))
    );
    assert_eq!(layout.usable_area, layout.target_workarea.unwrap());
    assert_eq!(
        layout.valid_origins,
        ValidOriginBounds::new(-1900, -260, -80, 660)
    );
}

#[test]
fn workarea_missing_extra_and_non_four_word_entries_are_rejected() {
    for length in [0, 4, 7, 9, 12] {
        let mut s = snapshot();
        s.workarea = property(&vec![1; length]);
        assert_unverifiable(validate(&s));
    }
}

#[test]
fn malformed_workarea_never_becomes_absent_fallback() {
    for p in malformed_replies(snapshot().workarea) {
        let mut s = snapshot();
        s.workarea = p;
        assert_unverifiable(validate(&s));
    }
}

#[test]
fn zero_workarea_width_and_height_are_rejected() {
    for area in [
        Rect::new(-1900, -80, 0, 900),
        Rect::new(-1900, -80, 1800, 0),
    ] {
        let mut s = snapshot();
        workarea(&mut s, area);
        assert_unverifiable(validate(&s));
    }
}

#[test]
fn workarea_right_and_bottom_must_be_representable() {
    for area in [
        Rect::new(i32::MAX, 0, 1, 900),
        Rect::new(0, i32::MAX, 1800, 1),
        Rect::new(0, 0, u32::MAX, 900),
        Rect::new(0, 0, 1800, u32::MAX),
    ] {
        let mut s = snapshot();
        workarea(&mut s, area);
        assert_unverifiable(validate(&s));
    }
}

#[test]
fn complete_workarea_validates_even_non_target_rectangles() {
    let mut s = snapshot();
    s.workarea = property(&[0, 0, 0, 300, (-1900_i32) as u32, 0, 1800, 900]);
    assert_unverifiable(validate(&s));
}

#[test]
fn cardinal_origins_use_signed_bits_while_dimensions_keep_full_unsigned_range() {
    let s = property(&[i32::MIN as u32, i32::MIN as u32, u32::MAX, u32::MAX]);
    let area = target_workarea(&s, 1, 0).unwrap().unwrap();
    assert_eq!(area, Rect::new(i32::MIN, i32::MIN, u32::MAX, u32::MAX));
    assert_eq!(area.right(), Ok(i32::MAX));
    assert_eq!(area.bottom(), Ok(i32::MAX));
    let mut s = snapshot();
    workarea(&mut s, area);
    assert_eq!(
        validate(&s).unwrap().usable_area,
        Rect::new(-1920, -100, 1920, 1080)
    );
}

#[test]
fn genuine_workarea_absence_uses_retained_monitor_bounds() {
    let mut s = snapshot();
    s.workarea = absent();
    let layout = validate(&s).unwrap();
    assert_eq!(layout.target_workarea, None);
    assert_eq!(layout.usable_area, Rect::new(-1920, -100, 1920, 1080));
    assert_eq!(
        layout.valid_origins,
        ValidOriginBounds::new(-1920, -160, -100, 820)
    );
}

#[test]
fn genuinely_absent_viewport_is_compatible_and_not_synthesized() {
    let mut s = snapshot();
    s.viewport = absent();
    assert_eq!(validate(&s).unwrap().viewport, ViewportEvidence::Absent);
}

#[test]
fn exact_zero_viewports_for_multiple_desktops_are_compatible() {
    assert_eq!(
        validate(&snapshot()).unwrap().viewport,
        ViewportEvidence::AllZero
    );
}

#[test]
fn nonzero_viewport_x_or_y_on_any_desktop_is_unsupported() {
    for index in 0..4 {
        let mut coordinates = [0; 4];
        coordinates[index] = if index == 0 { u32::MAX } else { 1 };
        let mut s = snapshot();
        s.viewport = property(&coordinates);
        assert_eq!(validate(&s), Err(DestinationError::UnsupportedViewport));
    }
}

#[test]
fn missing_extra_and_empty_viewport_entries_are_unverifiable() {
    for length in [0, 2, 3, 5, 6] {
        let mut s = snapshot();
        s.viewport = property(&vec![0; length]);
        assert_unverifiable(validate(&s));
    }
}

#[test]
fn viewport_type_format_truncation_and_metadata_are_not_unsupported() {
    for p in malformed_replies(property(&[0; 4])) {
        let mut s = snapshot();
        s.viewport = p;
        assert_unverifiable(validate(&s));
    }
    let mut s = snapshot();
    s.viewport = property(&[1, 0]); // Nonzero but incomplete: malformed takes precedence.
    assert_unverifiable(validate(&s));
}

#[test]
fn retained_atom_wins_over_primary_and_list_order() {
    let mut s = snapshot();
    s.monitors.reverse();
    assert_eq!(validate(&s).unwrap().monitor.name_atom, SELECTED);
}

#[test]
fn retained_monitor_disappearance_never_migrates() {
    let mut s = snapshot();
    s.monitors.remove(0);
    assert_unverifiable(validate(&s));
    s.monitors.clear();
    assert_unverifiable(validate(&s));
}

#[test]
fn duplicate_retained_identity_is_unverifiable() {
    let mut s = snapshot();
    s.monitors.push(s.monitors[0].clone());
    assert_unverifiable(validate(&s));
}

#[test]
fn empty_retained_monitor_dimensions_are_rejected_even_without_workarea() {
    for dimension in [0, 1] {
        let mut s = snapshot();
        s.workarea = absent();
        if dimension == 0 {
            s.monitors[0].width = 0;
        } else {
            s.monitors[0].height = 0;
        }
        assert_unverifiable(validate(&s));
    }
}

#[test]
fn empty_or_touching_monitor_workarea_intersection_is_rejected() {
    for area in [Rect::new(10, 20, 400, 300), Rect::new(0, -100, 400, 300)] {
        let mut s = snapshot();
        workarea(&mut s, area);
        assert_unverifiable(validate(&s));
    }
}

#[test]
fn target_workarea_is_intersected_with_retained_monitor() {
    let mut s = snapshot();
    workarea(&mut s, Rect::new(-2000, -200, 1000, 1000));
    let layout = validate(&s).unwrap();
    assert_eq!(layout.usable_area, Rect::new(-1920, -100, 920, 900));
    assert_eq!(
        layout.valid_origins,
        ValidOriginBounds::new(-1920, -1160, -100, 640)
    );
}

#[test]
fn exact_body_size_usable_area_has_one_valid_origin() {
    let mut s = snapshot();
    workarea(&mut s, Rect::new(-1800, 20, 160, 160));
    assert_eq!(
        validate(&s).unwrap().valid_origins,
        ValidOriginBounds::new(-1800, -1800, 20, 20)
    );
}

#[test]
fn one_pixel_too_narrow_or_short_cannot_fit_body() {
    for area in [
        Rect::new(-1800, 20, 159, 160),
        Rect::new(-1800, 20, 160, 159),
    ] {
        let mut s = snapshot();
        workarea(&mut s, area);
        assert_unverifiable(validate(&s));
    }
}

#[test]
fn body_size_is_supplied_and_zero_or_overflowing_sizes_cannot_fit() {
    let s = snapshot();
    let size = Size::new(300, 200);
    let layout = validate_snapshot(&s, SELECTED, TARGET, size).unwrap();
    assert_eq!(layout.body_size, size);
    assert_eq!(
        layout.valid_origins,
        ValidOriginBounds::new(-1900, -400, -80, 620)
    );
    for size in [
        Size::new(0, 160),
        Size::new(160, 0),
        Size::new(u32::MAX, 160),
        Size::new(160, u32::MAX),
    ] {
        assert_unverifiable(validate_snapshot(&s, SELECTED, TARGET, size));
    }
}

#[test]
fn identical_complete_snapshots_establish_agreement() {
    let s = snapshot();
    assert_eq!(pair(s.clone(), s.clone()), validate(&s));
}

#[test]
fn desktop_count_change_is_stale_even_with_complete_new_arrays() {
    let before = snapshot();
    let mut after = before.clone();
    after.count = property(&[3]);
    after.workarea = property(&[
        10,
        20,
        400,
        300,
        (-1900_i32) as u32,
        (-80_i32) as u32,
        1800,
        900,
        0,
        0,
        400,
        300,
    ]);
    after.viewport = property(&[0; 6]);
    assert_eq!(pair(before, after), Err(DestinationError::Changed));
}

#[test]
fn current_desktop_change_is_stale_without_chasing_it() {
    let before = snapshot();
    let mut after = before.clone();
    after.current = property(&[0]);
    assert_eq!(pair(before, after), Err(DestinationError::Changed));
}

#[test]
fn target_workarea_change_is_stale() {
    let before = snapshot();
    let mut after = before.clone();
    workarea(&mut after, Rect::new(-1899, -80, 1799, 900));
    assert_eq!(pair(before, after), Err(DestinationError::Changed));
}

#[test]
fn workarea_presence_change_is_stale_in_either_direction_even_with_same_usable_area() {
    let mut present = snapshot();
    workarea(&mut present, Rect::new(-1920, -100, 1920, 1080));
    let mut missing = present.clone();
    missing.workarea = absent();
    assert_eq!(
        validate(&present).unwrap().usable_area,
        validate(&missing).unwrap().usable_area
    );
    assert_eq!(
        pair(present.clone(), missing.clone()),
        Err(DestinationError::Changed)
    );
    assert_eq!(pair(missing, present), Err(DestinationError::Changed));
}

#[test]
fn viewport_zero_to_nonzero_is_unsupported_and_not_accepted_as_fresh() {
    let before = snapshot();
    let mut after = before.clone();
    after.viewport = property(&[0, 0, 1, 0]);
    assert_eq!(
        pair(before, after),
        Err(DestinationError::UnsupportedViewport)
    );
}

#[test]
fn viewport_presence_change_is_stale_in_either_direction() {
    let present = snapshot();
    let mut missing = present.clone();
    missing.viewport = absent();
    assert_eq!(
        pair(present.clone(), missing.clone()),
        Err(DestinationError::Changed)
    );
    assert_eq!(pair(missing, present), Err(DestinationError::Changed));
}

#[test]
fn retained_monitor_origin_changes_are_stale_even_if_usable_area_is_unchanged() {
    for axis in [0, 1] {
        let before = snapshot();
        let mut after = before.clone();
        if axis == 0 {
            after.monitors[0].x += 1;
        } else {
            after.monitors[0].y += 1;
        }
        assert_eq!(
            validate(&before).unwrap().usable_area,
            validate(&after).unwrap().usable_area
        );
        assert_eq!(pair(before, after), Err(DestinationError::Changed));
    }
}

#[test]
fn retained_monitor_size_changes_are_stale_even_if_usable_area_is_unchanged() {
    for dimension in [0, 1] {
        let before = snapshot();
        let mut after = before.clone();
        if dimension == 0 {
            after.monitors[0].width -= 1;
        } else {
            after.monitors[0].height -= 1;
        }
        assert_eq!(
            validate(&before).unwrap().usable_area,
            validate(&after).unwrap().usable_area
        );
        assert_eq!(pair(before, after), Err(DestinationError::Changed));
    }
}

#[test]
fn primary_changes_alone_do_not_invalidate_retained_evidence() {
    let before = snapshot();
    let mut after = before.clone();
    after.monitors[0].is_primary = true;
    after.monitors[1].is_primary = false;
    assert_eq!(pair(before.clone(), after), validate(&before));
}

#[test]
fn names_outputs_list_order_and_unrelated_monitors_are_not_layout_identity() {
    let before = snapshot();
    let mut after = before.clone();
    after.monitors[0].name = "renamed diagnostic label".into();
    after.monitors[0].outputs = vec![500, 600];
    after.monitors[1].x = 300;
    after.monitors[1].width = 800;
    after.monitors.reverse();
    assert_eq!(pair(before.clone(), after), validate(&before));
}

#[test]
fn non_target_workarea_changes_do_not_invalidate_target_evidence() {
    let before = snapshot();
    let mut after = before.clone();
    after.workarea = property(&[
        20,
        30,
        600,
        500,
        (-1900_i32) as u32,
        (-80_i32) as u32,
        1800,
        900,
    ]);
    assert_eq!(pair(before.clone(), after), validate(&before));
}

#[test]
fn malformed_first_or_second_snapshot_is_never_accepted() {
    let valid = snapshot();
    let mut malformed = valid.clone();
    malformed.workarea.bytes_after = 4;
    assert_unverifiable(pair(valid.clone(), malformed.clone()));
    assert_unverifiable(pair(malformed, valid));
}

#[test]
fn two_read_disagreement_never_retries_until_agreement() {
    let first = snapshot();
    let mut second = first.clone();
    second.monitors[0].x += 1;
    let mut calls = 0;
    let result = query_with(SELECTED, TARGET, BODY, || {
        calls += 1;
        match calls {
            1 => Ok(first.clone()),
            2 => Ok(second.clone()),
            _ => panic!("No retry/polling allowed, even if a third read would agree"),
        }
    });
    assert_eq!(result, Err(DestinationError::Changed));
    assert_eq!(calls, 2);
}

#[test]
fn native_read_errors_fail_without_retry() {
    for failed_read in [1, 2] {
        let mut calls = 0;
        let result = query_with(SELECTED, TARGET, BODY, || {
            calls += 1;
            assert!(calls <= failed_read, "No retry after read failure");
            if calls == failed_read {
                Err(unverifiable("injected native read failure"))
            } else {
                Ok(snapshot())
            }
        });
        assert_unverifiable(result);
        assert_eq!(calls, failed_read);
    }
}

#[test]
fn property_sequence_numbers_do_not_define_layout_freshness() {
    let before = snapshot();
    let mut after = before.clone();
    for reply in [
        &mut after.count,
        &mut after.current,
        &mut after.workarea,
        &mut after.viewport,
    ] {
        reply.sequence = u16::MAX;
    }
    assert_eq!(pair(before.clone(), after), validate(&before));
}
