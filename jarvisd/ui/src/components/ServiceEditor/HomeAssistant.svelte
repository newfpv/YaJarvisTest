<script module>
  let lastEntities = [];
</script>

<script>
  import { untrack } from "svelte";
  import { act, api, toast } from "#helpers/core.js";
  import { S } from "#state/panelState.js";

  let { state: snapshot, refresh } = $props();
  let current = $derived(snapshot.home_assistant || {});
  let address = $state(untrack(() => current.url || ""));
  let token = $state("");
  let rooms = $state(untrack(() => copyRooms(current.rooms)));
  let entities = $state.raw(lastEntities);

  function copyRooms(list) {
    return (list || []).map((room) => ({ ...room }));
  }

  function options(value, pattern) {
    const entries = new Map();
    if (value) entries.set(value, { entity_id: value });
    for (const entity of entities) {
      if (pattern.test(entity.entity_id)) entries.set(entity.entity_id, entity);
    }
    return [...entries.values()];
  }

  function addRoom() { rooms.push({ name: "", vacuum: "", entity: "", room_ids: "" }); }

  function removeRoom(index) { rooms.splice(index, 1); }

  async function loadEntities() {
    const data = await api("GET", "/api/home-assistant");
    entities = lastEntities = data.entities || [];
  }

  function load(event) {
    return act(event.currentTarget, async () => {
      await loadEntities();
      toast(`Получено устройств и действий: ${entities.length}`);
    });
  }

  function discover(event) {
    return act(event.currentTarget, async () => {
      const data = await api("POST", "/api/home-assistant/rooms/discover", {});
      if (!data.rooms?.length) {
        toast("Автоматическое определение комнат недоступно. Можно выбрать готовые сценарии уборки.", true);
        return;
      }
      await loadEntities();
      rooms = copyRooms(data.rooms);
      toast(`Найдено комнат: ${data.rooms.length}. Нажмите «Проверить и сохранить».`);
    });
  }

  function save(event) {
    return act(event.currentTarget, async () => {
      const bindings = rooms.map((room) => ({
        ...room, name: (room.name || "").trim(), vacuum: room.vacuum || "",
        entity: room.entity || "", room_ids: (room.room_ids || "").trim(),
      }));
      await api("PUT", "/api/home-assistant", { url: address, token, rooms: bindings });
      token = "";
      await refresh();
      rooms = copyRooms(S.home_assistant?.rooms);
    }, "Home Assistant подключён");
  }

  export function disconnect(event) {
    return act(event.currentTarget, async () => {
      await api("DELETE", "/api/home-assistant");
      entities = lastEntities = [];
      rooms = [];
      await refresh();
    }, "Home Assistant отключён");
  }
</script>

<p class="note">Подключите свой сервер Home Assistant. Аккаунт Xiaomi, регион и устройства выбираются в самом Home Assistant.</p>
<div class="f"><label for="ha-url">Адрес сервера</label><input id="ha-url" type="url" bind:value={address} placeholder="http://192.168.1.10:8123"></div>
<div class="f"><label for="ha-token">Долгосрочный токен из профиля Home Assistant</label>
  <input id="ha-token" type="password" autocomplete="off" bind:value={token} placeholder={current.configured ? "Оставьте пустым, чтобы сохранить текущий" : "Вставьте токен"}>
</div>
<div class="row">
  <button type="button" class="btn primary" id="ha-save" onclick={save}>Проверить и сохранить</button>
  {#if current.configured}
    <button type="button" class="btn" id="ha-load" onclick={load}>Загрузить устройства</button>
    <a href={current.url + "/config/integrations/dashboard"} target="_blank" rel="noopener">Открыть Home Assistant</a>
  {/if}
</div>
<p class="note">Для Xiaomi добавьте интеграцию Xiaomi Home и войдите в свой аккаунт. Название комнаты в приложении и номер зоны на карте пылесоса могут различаться.</p>
<h3>Уборка по комнатам</h3><p class="note">Выберите готовый сценарий уборки либо действие Xiaomi с номерами зон. Без соответствия Джарвис попросит настроить комнату.</p>
<div id="ha-rooms">
  {#each rooms as room, index (room)}
    <div class="card" data-ha-room>
      <div class="f"><label for={"ha-name-" + index}>Название комнаты</label><input id={"ha-name-" + index} class="ha-name" bind:value={room.name} placeholder="Кухня"></div>
      <div class="f"><label for={"ha-vacuum-" + index}>Пылесос</label><select id={"ha-vacuum-" + index} class="ha-vacuum" bind:value={room.vacuum}>
        <option value="">Выберите…</option>
        {#each options(room.vacuum, /^vacuum\./) as entity (entity.entity_id)}
          <option value={entity.entity_id}>{entity.attributes?.friendly_name || entity.entity_id}</option>
        {/each}
      </select></div>
      <div class="f"><label for={"ha-entity-" + index}>Сценарий или действие уборки этой комнаты</label><select id={"ha-entity-" + index} class="ha-entity" bind:value={room.entity}>
        <option value="">Выберите…</option>
        {#each options(room.entity, /^(script|notify|text)\./) as entity (entity.entity_id)}
          <option value={entity.entity_id}>{entity.attributes?.friendly_name || entity.entity_id}</option>
        {/each}
      </select></div>
      <div class="f"><label for={"ha-ids-" + index}>Номера зон на карте — только для действия Xiaomi</label><input id={"ha-ids-" + index} class="ha-ids" bind:value={room.room_ids} placeholder="Например, 3 или 3,9"></div>
      <button type="button" class="btn ghost" data-ha-remove onclick={() => removeRoom(index)}>Убрать комнату</button>
    </div>
  {/each}
</div>
<div class="row">
  <button type="button" class="btn" id="ha-discover-rooms" onclick={discover}>Найти комнаты пылесоса</button>
  <button type="button" class="btn" id="ha-add-room" onclick={addRoom}>Добавить комнату</button>
</div>
