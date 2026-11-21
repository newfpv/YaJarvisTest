<script module>
  const messageClasses = {
    day: "svc day",
    service: "svc",
    message: "b me",
    listening: "b me live",
    answer: "b him ans",
    thinking: "b him think",
    speaking: "b him speak",
    error: "b him err",
  };
  const messageLabels = {
    listening: "Говорите",
    thinking: "Джарвис думает",
    speaking: "Джарвис отвечает",
  };
</script>

<script>
  import { untrack } from "svelte";
  import { icon } from "#helpers/core.js";
  import { animateAnswer } from "./animateAnswer.js";

  let { row, animate, follow } = $props();
  const entering = untrack(() => animate && row.kind !== "answer");
  let node = $state();
  let previousAnswer;
  let previousSize;

  $effect.pre(() => {
    if (row.answer === previousAnswer) return;
    previousAnswer = row.answer;
    previousSize = node?.isConnected ? { w: node.offsetWidth, h: node.offsetHeight } : undefined;
  });
</script>

<div bind:this={node} class={messageClasses[row.kind]} class:new={entering && row.kind !== "answer"} aria-label={messageLabels[row.kind]}>
  {#if row.kind === "day" || row.kind === "service"}
    {row.text}
  {:else if row.kind === "message" || row.kind === "error"}
    <span class="t">{row.text}</span>
    <span class="m">
      {row.time}
      {#if row.kind === "message"}{@html icon(row.answered ? "read" : "sent")}{/if}
    </span>
  {:else if row.kind === "listening"}
    <span class="wave" aria-hidden="true"><i></i><i></i><i></i><i></i><i></i><i></i><i></i></span>
    <span class="t">Слушаю</span>
  {:else if row.kind === "thinking"}
    <span class="think-t">Думаю</span>
  {:else if row.kind === "speaking"}
    <span class="eqw" aria-hidden="true"><i></i><i></i><i></i><i></i><i></i></span>
    <span class="t">Отвечает</span>
  {:else if row.kind === "answer"}
    <span class="t" use:animateAnswer={{ text: row.answer, animate, follow, measure: () => previousSize }}></span>
    <span class="m">{row.time}</span>
  {/if}
</div>
