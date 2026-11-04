// Make sure bundled Manrope is ready before the first line is measured.
// Lines measured with a fallback face would be rebuilt when it arrives, so
// waiting (briefly) avoids a visible re-layout on start.

export async function preloadFonts(timeoutMs = 1500): Promise<void> {
  if (!document.fonts?.load) return;
  const loads = Promise.all([
    document.fonts.load('400 48px "Manrope Variable"', "Lyrics"),
    document.fonts.load('700 48px "Manrope Variable"', "Lyrics"),
  ]).catch(() => undefined);
  await Promise.race([loads, new Promise((resolve) => setTimeout(resolve, timeoutMs))]);
}
