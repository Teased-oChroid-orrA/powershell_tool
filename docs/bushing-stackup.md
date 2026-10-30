> Historical preliminary notes below are superseded by the Precision Studio update and verification record at the end of this document. Preliminary dimensions are not accepted designs.

# Bushing Stackup toolbox

The toolbox is available from the Toolbench dashboard as **Bushing Stackup**, opening a native window (`--gpu-bushing-stackup`). A terminal view remains available as `app-tui --toolbox bushing-stackup`. New engineering workbenches launch in native windows from the dashboard by owner preference.

The original hole has one straight axis through the full stack, bounded by the original angularity input. Line reaming makes one new straight axis through the same full stack, bounded independently by the cleaned-hole angularity input. Each member has an original hole diameter and thickness. The bushings are separate pieces; the optional common OD setting uses one required reamed-hole diameter for all members.

For the preliminary envelope, the pilot datum has an axial location from the stack entry face and a center-offset limit. The axes tilt in opposite directions. Their relative slope is `tan(original_angle) + tan(reamed_angle)`. The maximum center offset in a member is the pilot offset limit plus that slope times the larger absolute distance of its faces from the pilot datum. The cleanup diameter is `original_hole_diameter + 2 × maximum_center_offset`. The required reamed-hole diameter is the larger of the cleanup diameter and `bushing_ID_min + 2 × minimum_radial_wall`. This is not an accepted bushing OD or finished bore result.

The `r` action sends the selected member's required hole diameter, thickness, minimum bushing ID and minimum wall to the Bushing Workbench, then opens it. This sets the bore tolerance to zero as an exact geometric starting point; the user must choose manufacturing tolerance and fit there. Other Bushing Workbench inputs remain as they were.

This is preliminary geometric envelope sizing. It does not select a press fit, bushing OD manufacturing tolerance, material, or strength. The native transfer is blocked because the Bushing case cannot preserve the eccentricity and axis geometry; the terminal transfer remains legacy behavior and must not be used as evidence of a validated eccentric design. The schematic is explanatory and is not to scale. The calculation uses full precision until display rounding.

Verification: focused `app-tui` tests cover worst-case cleanup sizing, common diameter, invalid native-page input, dashboard native-window routing, and the terminal-to-Bushing handoff. The native binary launched on macOS and reported `FEA_GPU_READY`; the terminal view was inspected at 120×38 and 80×24.

## Precision Studio update (2026-10-29)

The current native implementation supersedes the preliminary workflow above. Bushing Stackup is native-only; it has no terminal transfer route. Current member hole diameter is an input. The calculated angularity-only cleanup diameter contains the original cylindrical hole on one common finished axis through the stack. The search includes interior finished-axis orientations as well as interval endpoints, and uses the member face bounds and pilot datum.

Optional bushing sizing can enlarge the calculated hole to meet the required radial wall. Results retain the angularity-only diameter separately and explicitly identify enlargement and its reason. With bushings disabled, only hole sizing applies. The native Bushing Workbench transfer preserves sizing context and blocks unsupported tilted geometry.

Precision Studio displays complete translucent members, original-hole outlines, cylindrical cleaned bores and hollow bushings. Fitted and exploded views use the same solver geometry; explosion translates components along the common finished axis. Members separate before bushings withdraw, with axial intervals kept separate in the final exploded view. Display offsets do not change calculations.

The supported uncertainty model is coplanar straight axes, with thickness measured along stack z. Cylindrical-zone mode uses a straight derived axis at RFS and a separate pilot-position constraint; it does not apply MMC/LMC bonus tolerance or datum mobility. Reference normals constrain the shared axis intervals rather than creating independently reamed member axes.

Structural transfer retains the source inputs, cleanup diameter, adopted hole and enlargement reason. The Bushing Workbench uses its existing plane FE geometry with an extended far boundary and editable load/fit defaults; its source card identifies that approximation. Oblique finished axes and unsupported ID tolerance bands are refused rather than discarded.

Verification on 2026-10-29 included the full bushing-solver suite, native session and transfer tests, and GPU renders at 1360×860 in both themes and 780×900 in the dark theme. The exploded-view regression checks 101 animation positions at three separations. An independent read-only review found no remaining source blockers after checking the common-axis envelope, interior-angle extrema, enlargement reporting, revision lifecycle, geometry and transfer provenance.

Final local qualification passed `cargo test -p app-tui --lib` with 1,230 passed and 25 ignored, and `cargo test -p bushing-solver` with 60 unit tests plus both integration tests passing. The ignored stackup GPU snapshot test was run separately with graphics access and passed. The calculation exercise completed for 2, 10, 100 and 1,000 members with individual and uniform bushing OD; its debug timings are smoke-check evidence only, not a performance comparison.

## Input and view interaction update (2026-10-29)

The inspector is the single editing surface, with **Member settings** and **Stack setup** tabs. The former Selected member and Stack configuration popovers no longer duplicate those controls. Member settings groups geometry, hole orientation, datum direction, machining limits and optional bushing policy. Stack setup contains pilot registration and shared sizing settings.

**Datum normal angle (deg)** is an angular datum measured from +Z toward +X. A value of 0.5 means 0.5 degrees. **Pilot position from stack entrance (in)** is a separate axial location; 0.5 inches is the midpoint of a single 1-inch member.

Enter or leaving an input commits calculation. Switching an inspector tab or closing a section commits an outstanding edit before hiding it. Invalid active text keeps results unavailable and structural transfer blocked. Discrete geometry and policy changes calculate immediately. Search inputs retain their existing Enter-only run behavior.

The view shows the common finished axis, original-axis segments that follow their members, and an orientation triad. The explosion slider spans fitted (0%) through fully exploded (100%). Dragging upward in the geometry view separates components; dragging downward assembles them. Both presets remain available. The view fits the current pose continuously and uses the same collision-safe separation schedule without changing engineering inputs or results.

Verification passed the app library suite with 1,234 tests and 25 ignored. After correcting compact inspector sizing, all 16 focused stackup/session/transfer/interaction tests passed; the separate GPU snapshot check passed with fitted, exploded and settings captures in dark/light themes and at 780×900. The compact interaction test scrolls to the inspector and edits both pilot registration fields. Snapshot capture asserts that the requested view or settings tab actually opened. The rebuilt macOS native application launched and reported `FEA_GPU_READY`; independent source review found no remaining production blockers. The existing Search inputs retain Enter-only submit behavior. Windows runtime interaction was not exercised locally.


## Individual component motion and shared-axis sizing (2026-10-29)

Drag a member, bushing, or its label to translate only that component along the common finished axis. The captured component remains the drag target until release. Its own member/bushing pair may overlap axially because the bushing nests in its bore. Motion stops at components from other members; it does not push neighbors or jump through them. Internal members may therefore need their neighbors moved first. Blank-canvas dragging does nothing. **Fitted** resets all display positions, and **Explode all** explicitly separates the whole assembly. Display positions do not enter engineering calculations or saved case inputs. A geometry change resets incompatible display positions; a name edit or member selection retains them.

The app retains one shared original axis and one shared finished axis through the entire stack. Each member constrains the permitted ranges, and the intersection of those constraints governs all members. Stack setup now shows the effective ranges. For a 0.500-inch hole through a 1-inch first member with original angularity ±2°, finished angularity 0° and the pilot at entry, angularity-only cleanup is 0.570146311133 inches. Adding a member with ±1° original angularity narrows the common original range to ±1° and reduces the first member's cleanup to 0.534986293878 inches. This is a shared-constraint effect, not total stack thickness being incorrectly applied to the first member. Identical added angular constraints leave its cleanup unchanged.

New members inherit the selected member's datum normal and original/finished angularity definitions, including cylindrical-zone dimensions and reference spans. Their other dimensions retain the normal new-member defaults. Existing cases are unchanged, and deliberately entering a tighter member constraint can still change the shared ranges. Uniform bushing OD can increase the first member's adopted hole to match a downstream member, while its angularity-only cleanup remains separately reported.
