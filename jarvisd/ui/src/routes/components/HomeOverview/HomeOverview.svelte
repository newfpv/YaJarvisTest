<script>
  import { icon, tip } from "#helpers/core.js";

  let { overview } = $props();
  let completed = $derived(overview.steps.filter(({ done }) => done).length);
</script>

{#if completed < overview.steps.length}
  <div class="card">
    <h2>Осталось настроить <span class="count">{completed} из {overview.steps.length}</span></h2>
    <progress class="bar" value={completed} max={overview.steps.length} aria-label="Настройка колонки"></progress>
    {#each overview.steps as step (step.title)}
      <a class="step" class:done={step.done} href={step.href}>
        {@html icon(step.done ? "done" : "todo")}
        <span class="sb"><b>{step.title}</b><small>{step.text}</small></span>
        {#if !step.done}{@html icon("right")}{/if}
      </a>
    {/each}
  </div>
{/if}

<div class="card">
  <h2>Состояние</h2>
  <dl class="stat">
    {#each overview.statuses as status (status.id)}
      <div>
        <dt>{status.label} {@html tip(status.tip)}</dt>
        <dd class="v {status.level}" id={status.id}>{status.text}</dd>
      </div>
    {/each}
  </dl>
</div>
