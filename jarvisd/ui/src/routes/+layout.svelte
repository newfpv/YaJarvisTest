<script>
  import App from "#components/App.svelte";
  import "../app.css";

  let { children } = $props();
</script>

<App {children} />
