import { describe, expect, it } from "vitest";
import { gazeFromPointer } from "./VrmCanvas";

const rect = { left: 100, top: 50, width: 200, height: 100 };

describe("desktop pet gaze", () => {
  it("maps the center of the pet window to a neutral gaze", () => {
    expect(gazeFromPointer(200, 100, rect)).toEqual({ x: 0, y: 0 });
  });

  it("maps pointer position to a bounded gaze target", () => {
    expect(gazeFromPointer(150, 75, rect)).toEqual({ x: -0.5, y: 0.5 });
    expect(gazeFromPointer(500, -100, rect)).toEqual({ x: 1, y: 1 });
  });

  it("falls back to neutral for invalid geometry", () => {
    expect(gazeFromPointer(200, 100, { ...rect, width: 0 })).toEqual({ x: 0, y: 0 });
  });
});
