# Авторы и благодарности

YaJavis появился благодаря открытым проектам и исследованиям сообщества. Ниже указано, какие части их работы использовались как основа, ориентир или техническая документация. Авторство сторонних компонентов сохраняется за их создателями.

## Яндекс Станции и интерфейс

| Автор | Проект | Что использовано |
| :--- | :--- | :--- |
| **dimap** | [yapanel](https://github.com/dimap/yapanel-web) | Визуальный ориентир веб-панели: тёмная тема, оранжевый акцент, карточки состояния, графики и музыкальный интерфейс. Атрибуция также сохранена в заголовке `jarvisd/ui/src/app.css`. |
| **Marker284 и участники сообщества** | [Yandex Station Extended](https://github.com/Marker284/yandex-station-extended), [hackalice.me](https://hackalice.me/) | Исследования root-доступа, устройства прошивок, аудиотракта, подсветки и аппаратных различий Станций. |
| **rison1337** | [Alisa ChatGPT Voice Assistant](https://github.com/rison1337/Alisa-ChatGPT-voice-assistant) | Отправная точка ранних экспериментов с ChatGPT Voice на Станции и подключением аудио колонки. |
| **amirilivice** | [quasard](https://github.com/amirilivice/quasard) | Ориентир на раннем этапе работы с собственным голосовым бэкендом Станции. Текущая программа YaJavis работает непосредственно на колонке. |
| **tangenx** | [station-wakeword](https://github.com/tangenx/station-wakeword) | Технический ориентир для локальной активации и подхода с признаками openWakeWord. Чужие наборы аудиозаписей не включены в репозиторий. |
| **Morvex885** | [Pulsar](https://github.com/Morvex885/pulsar) | Документация аппаратной части и штатного распознавателя коротких команд Станции. |
| **nyakokitsu** | [YAM](https://github.com/nyakokitsu/yam) | Документация идентификаторов аппаратных платформ и особенностей определения моделей. |

Перечисленные проекты не являются авторами или официальными сопровождающими YaJavis. Описание вклада не означает, что каждый проект целиком включён в поставку.

## Распознавание, звук и инфраструктура

В качестве визуальных референсов движения установщика использовалась подборка [DesignBookmark Motion](https://designbookmark.com/motion): переходы шагов, отклик кнопок и смена состояний. Реализация использует CSS и Web Animations API вместе с анимациями самой панели YaJavis.

| Авторы | Компонент | Роль |
| :--- | :--- | :--- |
| **dscripka и участники openWakeWord; Google** | [openWakeWord](https://github.com/dscripka/openWakeWord) | Мел-признаки и speech embedding для локального слова активации. |
| **Команды yt-dlp и EJS** | [yt-dlp/ejs](https://github.com/yt-dlp/ejs) | Подход к обработке скрипта YouTube и расшифровке параметров аудиоссылок. |
| **dop251 и участники goja** | [goja](https://github.com/dop251/goja) | Выполнение JavaScript на Go. |
| **XTLS и участники Xray-core** | [Xray-core](https://github.com/XTLS/Xray-core) | VLESS и сетевой транспорт. |
| **Pion** | [WebRTC](https://github.com/pion/webrtc) | Передача звука в голосовых сессиях. |
| **thesyncim** | [gopus](https://github.com/thesyncim/gopus) | Декодирование аудио Opus. |
| **Coder** | [websocket](https://github.com/coder/websocket) | WebSocket-соединения. |
| **Michael Sharanda и участники Manrope** | [Manrope](https://github.com/google/fonts/tree/main/ofl/manrope) | Шрифт веб-панели. |
| **Lucide contributors** | [Lucide](https://github.com/lucide-icons/lucide) | SVG-контуры иконок панели. |

Тексты лицензий и сведения о зависимостях находятся в [THIRD_PARTY.md](THIRD_PARTY.md) и [LICENSES](LICENSES). Имена, уведомления об авторстве и лицензии сохраняются вместе с соответствующими компонентами.
