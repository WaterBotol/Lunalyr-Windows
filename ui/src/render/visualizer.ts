// Visualiser background (Milestone 2). Until it is implemented the layer is
// never active and the colour field is drawn instead.

import type { AudioFrame, Settings } from "../contract";
import type { ViewportMetrics } from "../timing/stage";
import type { RGB } from "./palette";

export class VisualizerLayer {
  constructor(_parent: HTMLElement) {}

  get active(): boolean {
    return false;
  }

  configure(_settings: Settings): void {}

  resize(_width: number, _height: number): void {}

  setPaused(_paused: boolean): void {}

  pushAudio(_frame: AudioFrame): void {}

  render(_now: number, _flow: number, _stops: readonly RGB[], _settings: Settings, _metrics: ViewportMetrics): void {}
}

/** Whether the visualiser background can be chosen in this build. */
export const VISUALIZER_AVAILABLE = false;
