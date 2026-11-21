function formatUptime(seconds) {
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor(seconds % 86400 / 3600);
  const minutes = Math.floor(seconds % 3600 / 60);
  if (days) return `${days} д ${hours} ч`;
  if (hours) return `${hours} ч ${minutes} мин`;
  return `${minutes} мин`;
}

function setupSteps({ keys, chatgpt, profiles, active, text_plugins, vless, settings }) {
  const profile = profiles.find((item) => item.id === active);
  const hasTextPlugin = (text_plugins || []).some(({ id }) => profile?.llm_engine === id || profile?.llm_plugin === id);
  return [
    {
      done: Boolean(keys.openai || keys.gemini || keys.grok || chatgpt.signed_in || hasTextPlugin),
      title: "Подключить модель", text: "Выберите готовый сервис или установите свой текстовый плагин", href: "/connections",
    },
    {
      done: vless.set && !vless.error,
      title: "VPN (ссылка VLESS)", text: "OpenAI не работает напрямую из России и Беларуси", href: "/connections",
    },
    {
      done: Boolean(keys.yandex || settings.home_via !== "jarvis"),
      title: "Умный дом", text: "Через Алису работает сразу; для режима «Джарвис» нужен токен Яндекса", href: "/speaker",
    },
    { done: Boolean(settings.city), title: "Ваш город", text: "Для погоды и новостей без уточнений", href: "/speaker" },
  ];
}

export function readOverview(state) {
  const { speaker, settings, sys = {}, vless, version } = state;
  const title = `${speaker.model || "Яндекс Станция"} · Джарвис ${version}`;
  const uptime = sys.uptime_s ? ` · колонка работает ${formatUptime(sys.uptime_s)}` : "";
  return {
    summary: title + uptime,
    micOnline: speaker.mic,
    steps: setupSteps(state),
    statuses: [
      {
        id: "t-mic", label: "Микрофон", text: speaker.mic ? "на связи" : "нет звука", level: speaker.mic ? "ok" : "bad",
        tip: "Звук с микрофонов колонки после шумо- и эхоподавления Яндекса.",
      },
      {
        id: "t-wake", label: `Фраза «${settings.wake_word || "Джарвис"}»`,
        text: !settings.wake_enabled ? "выключено" : speaker.wake_model ? "слушаю" : "нет модели",
        level: settings.wake_enabled && speaker.wake_model ? "ok" : "warn",
        tip: "Распознаётся прямо на колонке маленькой нейросетью — без интернета и сразу.",
      },
      {
        id: "t-net", label: "Интернет", text: vless.error ? "ошибка VPN" : vless.set ? "через VPN" : "напрямую",
        level: vless.error ? "bad" : vless.set ? "ok" : "warn",
        tip: "Как колонка ходит в OpenAI: через ваш VPN (VLESS) или напрямую.",
      },
      {
        id: "t-disk", label: "Свободно на диске", text: sys.disk_total_mb ? `${sys.disk_free_mb} из ${sys.disk_total_mb} МБ` : "—",
        level: sys.disk_total_mb && sys.disk_free_mb < 5 ? "warn" : "ok",
        tip: "На флешке колонки мало места. Если свободно меньше 5 МБ, обновления могут не встать.",
      },
    ],
  };
}
