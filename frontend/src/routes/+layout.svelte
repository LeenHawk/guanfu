<script lang="ts">
  import "../app.css";
  import "$lib/styles/chat.css";
  import "$lib/styles/invocation.css";
  import "$lib/styles/responsive.css";

  let { children } = $props();
</script>

{@render children()}
