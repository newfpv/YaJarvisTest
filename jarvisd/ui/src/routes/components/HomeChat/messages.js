const services = new Map([
  ["умный дом (быстро)", "Выполнил сам"],
  ["умный дом → Алиса", "Передал Алисе"],
  ["стоп", "Остановлен"],
  ["ложное срабатывание", "Показалось, что позвали"],
]);

const dateFormat = new Intl.DateTimeFormat("ru-RU", { day: "numeric", month: "long" });
const timeFormat = new Intl.DateTimeFormat("ru-RU", { hour: "2-digit", minute: "2-digit" });
const localDay = (date) => new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();

function dayLabel(date, today) {
  const daysAgo = Math.round((today - localDay(date)) / 864e5);
  if (daysAgo === 0) return "Сегодня";
  if (daysAgo === 1) return "Вчера";
  return dateFormat.format(date);
}

function serviceText(turn, label) {
  if (turn.route === "умный дом → Алиса" && turn.answer) return turn.answer.replace(/^передал/, "Передал");
  return turn.answer && turn.answer !== "Готово." ? label + " · " + turn.answer : label;
}

export function chatMessages(journal, phase) {
  const turns = journal.filter((turn) => turn.route !== "ложное срабатывание" || turn.text).slice(0, 40);
  const rows = [];
  const today = localDay(new Date());
  const timestamps = new Map();
  let previousDay;
  for (const turn of turns.reverse()) {
    const date = new Date(turn.ts * 1000);
    if (!Number.isFinite(date.getTime())) continue;
    const day = localDay(date);
    const time = timeFormat.format(date);
    let id = turn.id;
    if (id == null) {
      const occurrence = timestamps.get(turn.ts) || 0;
      timestamps.set(turn.ts, occurrence + 1);
      id = `t${turn.ts}:${occurrence}`;
    }
    if (day !== previousDay) {
      rows.push({ key: "day:" + id, kind: "day", text: dayLabel(date, today) });
      previousDay = day;
    }
    const service = services.get(turn.route);
    const answered = Boolean(turn.answer || turn.error || service);
    if (turn.text) {
      rows.push({ key: "me:" + id, kind: "message", text: turn.text, time, answered });
    } else if (turn.live && !answered) {
      rows.push({ key: "me:" + id, kind: "listening" });
    }
    if (service) {
      rows.push({ key: "service:" + id, kind: "service", text: serviceText(turn, service) });
    } else if (turn.answer) {
      const delay = typeof turn.audio_s === "number" && turn.audio_s > 0 ? `ответ через ${turn.audio_s.toFixed(1)} с · ` : "";
      rows.push({ key: "him:" + id, kind: "answer", answer: turn.answer, time: delay + time });
    } else if (turn.live && !answered && turn.text) {
      rows.push({ key: "him:" + id, kind: phase === "speaking" ? "speaking" : "thinking" });
    }
    if (turn.error) rows.push({ key: "error:" + id, kind: "error", text: turn.error, time });
  }
  return rows;
}
