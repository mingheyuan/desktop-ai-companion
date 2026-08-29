import { describe, expect, it } from "vitest";
import { formatTimerRemaining } from "./timerFormat";

describe("formatTimerRemaining", () => {
  it("uses seconds and minutes for short reminders", () => {
    expect(formatTimerRemaining(60_000, 1_000)).toBe("剩余约59秒");
    expect(formatTimerRemaining(121_000, 1_000)).toBe("剩余约2分钟");
  });

  it("uses hours and days for longer reminders", () => {
    expect(formatTimerRemaining(3_601_000, 1_000)).toBe("剩余约1小时");
    expect(formatTimerRemaining(90_061_000, 1_000)).toBe("剩余约1天1小时");
  });

  it("does not show a negative countdown after the due time", () => {
    expect(formatTimerRemaining(1_000, 2_000)).toBe("即将触发");
  });
});
