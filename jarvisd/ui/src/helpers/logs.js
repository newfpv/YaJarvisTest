function logLevel(line) {
  if (/ошибк|error|fail|не получилось|panic|не удалось|не открылся/i.test(line)) return "bad";
  if (/warn|предупрежд|повтор|ложное/i.test(line)) return "warn";
  return "";
}

export function parseLogLine(line) {
  const match = /^(\d{4}\/\d\d\/\d\d |\[[\d:]+\] \[)?(\S*\s?\d\d:\d\d:\d\d(?:\.\d+)?\]?)?(.*)$/.exec(line) || [];
  return {
    time: (match[1] || "") + (match[2] || ""),
    message: match[3] ?? line,
    level: logLevel(line),
  };
}

// Короткая строка для главной: время — только минуты и секунды.
export function parseShortLogLine(line) {
  const match = /^\d{4}\/\d\d\/\d\d \d\d:(\d\d:\d\d)(?:\.\d+)? (.*)$/.exec(line);
  if (!match) return parseLogLine(line);
  const [, time, message] = match;
  return { time, message, level: logLevel(message) };
}
