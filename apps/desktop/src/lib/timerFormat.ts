export function formatTimerRemaining(dueAtMs: number, nowMs: number): string {
  const remainingMs = dueAtMs - nowMs;
  if (remainingMs <= 0) return "即将触发";

  const totalSeconds = Math.ceil(remainingMs / 1_000);
  if (totalSeconds < 60) return `剩余约${totalSeconds}秒`;

  const totalMinutes = Math.ceil(totalSeconds / 60);
  if (totalMinutes < 60) return `剩余约${totalMinutes}分钟`;

  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  if (hours < 24) {
    return minutes === 0 ? `剩余约${hours}小时` : `剩余约${hours}小时${minutes}分钟`;
  }

  const days = Math.floor(hours / 24);
  const remainingHours = hours % 24;
  return remainingHours === 0
    ? `剩余约${days}天`
    : `剩余约${days}天${remainingHours}小时`;
}
