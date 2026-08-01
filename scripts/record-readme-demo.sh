#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly SCRIPT_DIR
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
readonly REPO_ROOT
readonly FRAME_RATE=8
readonly OUTPUT_WIDTH=960

output_path="${1:-docs/assets/strek-showcase.gif}"
if [[ "$output_path" != /* ]]; then
    output_path="$REPO_ROOT/$output_path"
fi

for command in cargo ffmpeg jq magick; do
    if ! command -v "$command" >/dev/null 2>&1; then
        echo "record-readme-demo: missing required command: $command" >&2
        exit 1
    fi
done

cd "$REPO_ROOT"
cargo build -p strek --locked

demo_tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/strek-readme-demo.XXXXXX")"
readonly demo_tmp_dir
readonly frames_dir="$demo_tmp_dir/frames"
readonly socket_path="$demo_tmp_dir/automation.sock"
readonly app_log="$demo_tmp_dir/strek.log"
readonly strek_bin="$REPO_ROOT/target/debug/strek"
mkdir -p "$frames_dir"

demo_app_pid=""
cleanup() {
    local status=$?
    trap - EXIT INT TERM
    if [[ -n "$demo_app_pid" ]] && kill -0 "$demo_app_pid" 2>/dev/null; then
        kill "$demo_app_pid" 2>/dev/null || true
        wait "$demo_app_pid" 2>/dev/null || true
    fi
    if (( status != 0 )) && [[ -f "$app_log" ]]; then
        echo "record-readme-demo: Strek log follows" >&2
        sed -n '1,200p' "$app_log" >&2
    fi
    if [[ "${STREK_DEMO_KEEP_TEMP:-0}" == 1 ]]; then
        echo "record-readme-demo: preserved temporary files at $demo_tmp_dir" >&2
    else
        rm -rf -- "$demo_tmp_dir"
    fi
    exit "$status"
}
trap cleanup EXIT INT TERM

# Background mode parks a two-pixel window edge on the rightmost display. That
# keeps macOS from suspending GPUI's display link without activating Strek.
STREK_CONFIG_DIR="$demo_tmp_dir/config" \
    STREK_AUTOMATION_SOCKET="$socket_path" \
    "$strek_bin" --background >"$app_log" 2>&1 &
demo_app_pid=$!

automate() {
    STREK_AUTOMATION_SOCKET="$socket_path" "$strek_bin" automate "$@"
}

ready=false
for ((attempt = 0; attempt < 100; attempt += 1)); do
    if automate state >/dev/null 2>&1; then
        ready=true
        break
    fi
    if ! kill -0 "$demo_app_pid" 2>/dev/null; then
        echo "record-readme-demo: Strek exited before automation became ready" >&2
        exit 1
    fi
    sleep 0.1
done
if [[ "$ready" != true ]]; then
    echo "record-readme-demo: timed out waiting for Strek automation" >&2
    exit 1
fi
run_action() {
    automate "$@" >/dev/null
    sleep 0.1
}

frame_index=0
capture_hold() {
    local count=$1
    local source="$demo_tmp_dir/current.png"
    automate screenshot "$source" >/dev/null
    for ((copy = 0; copy < count; copy += 1)); do
        printf -v frame_name 'frame-%04d.png' "$frame_index"
        cp "$source" "$frames_dir/$frame_name"
        frame_index=$((frame_index + 1))
    done
}

selected_layer_id() {
    automate document | jq -er '
        [.document.layers[] | select(.selected)]
        | if length == 1 then .[0].id else error("expected one selected layer") end
    '
}

drag_pointer() {
    local start_x=$1
    local start_y=$2
    local end_x=$3
    local end_y=$4
    local steps=$5

    run_action pointer down "$start_x" "$start_y"
    for ((step = 1; step <= steps; step += 1)); do
        local x=$((start_x + (end_x - start_x) * step / steps))
        local y=$((start_y + (end_y - start_y) * step / steps))
        run_action pointer move "$x" "$y"
        capture_hold 1
    done
    run_action pointer up "$end_x" "$end_y"
    capture_hold 4
}

run_action ui layers-panel show
run_action ui design-panel show
run_action action view.zoom_to_fit
capture_hold 10

document_json="$(automate document)"
ink="$(jq -er '.document.saved_colors[] | select(.name == "Ink") | .color' <<<"$document_json")"
paper="$(jq -er '.document.saved_colors[] | select(.name == "Paper") | .color' <<<"$document_json")"
blue="$(jq -er '.document.saved_colors[] | select(.name == "Electric Blue") | .color' <<<"$document_json")"
magenta="$(jq -er '.document.saved_colors[] | select(.name == "Magenta") | .color' <<<"$document_json")"
mint="$(jq -er '.document.saved_colors[] | select(.name == "Mint") | .color' <<<"$document_json")"
amber="$(jq -er '.document.saved_colors[] | select(.name == "Amber") | .color' <<<"$document_json")"

# Precision aids: expose the starter project's snapping, grid, and guide system.
run_action ui precision-controls show
capture_hold 8
run_action ui precision-controls hide
capture_hold 3

# Draw and color a nested artboard. Pointer-move captures make the creation
# preview itself visible instead of jumping directly to the finished result.
run_action action tool.frame
capture_hold 2
drag_pointer 372 198 708 534 8
frame_id="$(selected_layer_id)"
run_action layer "$frame_id" --name "Automation Frame"
run_action color frame-background "$ink"
capture_hold 6

# Create two components inside the frame and apply colors from the project's
# saved palette.
run_action action tool.rectangle
drag_pointer 420 255 505 325 6
rectangle_id="$(selected_layer_id)"
run_action layer "$rectangle_id" --name "Launch Tile"
run_action color fill "$blue"
capture_hold 4

run_action action tool.ellipse
drag_pointer 545 255 620 330 6
ellipse_id="$(selected_layer_id)"
run_action layer "$ellipse_id" --name "Signal Dot"
run_action color fill "$magenta"
capture_hold 4

# The modeless picker exposes the document's Foundation and Signals libraries.
run_action ui fill-color-picker show
capture_hold 9
run_action ui fill-color-picker hide
run_action color fill "$amber"
capture_hold 4
run_action color fill "$magenta"
capture_hold 3

# Add a stroked path and live text to round out the tool showcase.
run_action action tool.line
drag_pointer 425 370 625 370 6
line_id="$(selected_layer_id)"
run_action layer "$line_id" --name "Connector"
run_action color stroke "$mint"
run_action property stroke-width 4
capture_hold 4

run_action action tool.text
drag_pointer 425 438 650 482 5
run_action text "BUILT BY AUTOMATION"
capture_hold 4
run_action action path.finish
text_id="$(selected_layer_id)"
run_action layer "$text_id" --name "Automation Label"
run_action color fill "$paper"
capture_hold 5

# Group the shapes, rename the component, and animate a duplicate moving into
# place. The Layers panel and canvas both reflect each structural edit.
run_action action tool.select
run_action select replace "$rectangle_id" "$ellipse_id"
capture_hold 3
run_action action arrange.group
group_id="$(selected_layer_id)"
run_action layer "$group_id" --name "Launch System"
capture_hold 7

run_action action edit.duplicate
for ((step = 0; step < 7; step += 1)); do
    run_action action selection.nudge_down_large
    capture_hold 1
done
capture_hold 3

run_action action arrange.ungroup
capture_hold 6
run_action color fill "$amber"
capture_hold 5

# Remove the duplicate, then its source, then the temporary frame. This makes
# deletion and nested-frame cleanup unmistakable before the document resets.
run_action action edit.delete
capture_hold 5
run_action select replace "$group_id"
run_action action edit.delete
capture_hold 5
run_action select replace "$line_id" "$text_id"
run_action action edit.delete
capture_hold 5
run_action select replace "$frame_id"
run_action action edit.delete
capture_hold 7

run_action ui command-palette show
capture_hold 8
run_action ui command-palette hide

for ((undo = 0; undo < 64; undo += 1)); do
    state_json="$(automate state)"
    if [[ "$(jq -r '.state.dirty' <<<"$state_json")" == false ]]; then
        break
    fi
    run_action action edit.undo
done
if [[ "$(automate state | jq -r '.state.dirty')" != false ]]; then
    echo "record-readme-demo: could not restore the starter document" >&2
    exit 1
fi
run_action action tool.select
capture_hold 10

readonly palette_path="$demo_tmp_dir/palette.png"
readonly unoptimized_path="$demo_tmp_dir/showcase-unoptimized.gif"
readonly optimized_path="$demo_tmp_dir/showcase.gif"
mkdir -p "$(dirname "$output_path")"
ffmpeg -hide_banner -loglevel error -y \
    -framerate "$FRAME_RATE" \
    -i "$frames_dir/frame-%04d.png" \
    -vf "scale=${OUTPUT_WIDTH}:-1:flags=lanczos,palettegen=max_colors=144:stats_mode=diff" \
    -frames:v 1 \
    "$palette_path"
ffmpeg -hide_banner -loglevel error -y \
    -framerate "$FRAME_RATE" \
    -i "$frames_dir/frame-%04d.png" \
    -i "$palette_path" \
    -lavfi "scale=${OUTPUT_WIDTH}:-1:flags=lanczos[x];[x][1:v]paletteuse=dither=bayer:bayer_scale=3:diff_mode=rectangle" \
    -loop 0 \
    "$unoptimized_path"

magick "$unoptimized_path" -coalesce -layers Optimize "$optimized_path"
mv "$optimized_path" "$output_path"

echo "Recorded $frame_index frames to $output_path"
