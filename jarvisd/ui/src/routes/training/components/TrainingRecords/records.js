export const recordsUrl = "/api/train/records";
export const clipKey = ({ storage, kind, name }) => [storage, kind, name].join("/");
export const clipUrl = ({ kind, name }) => recordsUrl + "/clip?" + new URLSearchParams({ kind, name });
export const clipWhen = ({ time }) => new Date(time).toLocaleString("ru-RU", {
  day: "numeric", month: "short", hour: "2-digit", minute: "2-digit",
});
export const clipProb = ({ name }) => name.match(/_p(\d+)/)?.[1];
export const clipSec = ({ size }) => Math.max(0, (size - 44) / 32000);
export const needsReview = ({ reviewed, kind }) => !reviewed && kind !== "bg";

const priority = { near: 0, pos: 1, false: 2 };

export function reviewQueue(clips, skipped) {
  return clips.filter((clip) => needsReview(clip) && !skipped.has(clipKey(clip)))
    .sort((a, b) => priority[a.kind] - priority[b.kind] || Date.parse(b.time) - Date.parse(a.time));
}
