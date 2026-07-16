<script>
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import Jauge from "./Jauge.svelte";
  import { duree } from "./temps.js";

  let releve = $state(null);
  let maintenant = $state(Math.floor(Date.now() / 1000));
  let occupe = $state(false);

  onMount(() => {
    // Rust pousse un instantane a chaque tour de boucle. On en redemande un a
    // l'ouverture, sinon la popover resterait vide jusqu'au tour suivant.
    const arret = listen("snapshot", (evt) => (releve = evt.payload));
    invoke("current").then((snap) => (releve = snap));

    // Le compte a rebours avance tout seul, en local : aucun appel reseau, et
    // surtout aucun risque d'ouvrir une fenetre de session pour afficher l'heure.
    const horloge = setInterval(() => (maintenant = Math.floor(Date.now() / 1000)), 1000);

    return () => {
      arret.then((detacher) => detacher());
      clearInterval(horloge);
    };
  });

  /** Sonde volontaire : c'est le seul chemin qui peut ouvrir une fenetre de 5h. */
  async function rafraichir() {
    if (occupe) return;
    occupe = true;
    try {
      releve = await invoke("refresh");
    } finally {
      occupe = false;
    }
  }

  function auClavier(evt) {
    if (evt.key === "Escape") invoke("hide");
    if (evt.key === "r" && (evt.metaKey || evt.ctrlKey)) rafraichir();
  }

  let age = $derived(
    releve?.probed_at != null ? maintenant - releve.probed_at : null,
  );
</script>

<svelte:window on:keydown={auClavier} />

<main>
  <header>
    <div class="identite">
      <span class="marque">Claude</span>
      {#if releve?.plan}
        <span class="plan">{releve.plan}</span>
      {/if}
    </div>
    <button
      class="rafraichir"
      class:tourne={occupe}
      onclick={rafraichir}
      disabled={occupe}
      title="Sonder maintenant. A froid, cela ouvre une fenetre de session de 5h."
      aria-label="Rafraichir"
    >
      <svg viewBox="0 0 16 16" aria-hidden="true">
        <path
          d="M13.5 8a5.5 5.5 0 1 1-1.6-3.9M13.5 2v3h-3"
          fill="none"
          stroke="currentColor"
          stroke-width="1.6"
          stroke-linecap="round"
          stroke-linejoin="round"
        />
      </svg>
    </button>
  </header>

  {#if releve?.windows?.length}
    <div class="jauges">
      {#each releve.windows as fenetre (fenetre.key)}
        <Jauge {fenetre} {maintenant} />
      {/each}
    </div>
  {:else}
    <div class="vide">
      <p>Aucun releve.</p>
      <p class="pourquoi">
        Claude Code n'a pas servi depuis plus de 5 h. Sonder maintenant ouvrirait
        une fenetre de session pour rien.
      </p>
      <button class="sonder" onclick={rafraichir} disabled={occupe}>
        Sonder quand meme
      </button>
    </div>
  {/if}

  <footer>
    {#if age != null}
      <span>{age < 60 ? "releve a l'instant" : `releve il y a ${duree(age)}`}</span>
    {/if}
    {#if releve?.origin === "probed"}
      <span class="point vert-point" title="releve frais"></span>
    {/if}
    {#if releve?.note}
      <span class="note" title={releve.note}>!</span>
    {/if}
  </footer>
</main>

<style>
  :global(:root) {
    --vert: #3fb950;
    --ambre: #d29922;
    --rouge: #f85149;
    --gris: #6e7681;

    --fond: rgba(255, 255, 255, 0.86);
    --encre: #1c2128;
    --encre-douce: #57606a;
    --trace: rgba(0, 0, 0, 0.09);
    --bord: rgba(0, 0, 0, 0.08);
  }

  @media (prefers-color-scheme: dark) {
    :global(:root) {
      --fond: rgba(28, 33, 40, 0.9);
      --encre: #e6edf3;
      --encre-douce: #8b949e;
      --trace: rgba(255, 255, 255, 0.1);
      --bord: rgba(255, 255, 255, 0.08);
    }
  }

  :global(body) {
    margin: 0;
    /* La fenetre est transparente : c'est `main` qui dessine le fond. */
    background: transparent;
    font-family:
      -apple-system, BlinkMacSystemFont, "Segoe UI", Inter, system-ui, sans-serif;
    -webkit-font-smoothing: antialiased;
    user-select: none;
    cursor: default;
    overflow: hidden;
  }

  main {
    box-sizing: border-box;
    height: 100vh;
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    background: var(--fond);
    backdrop-filter: blur(24px) saturate(180%);
    border: 1px solid var(--bord);
    border-radius: 12px;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding-bottom: 8px;
    border-bottom: 1px solid var(--trace);
  }

  .identite {
    display: flex;
    align-items: baseline;
    gap: 6px;
  }

  .marque {
    font-size: 13px;
    font-weight: 700;
    color: var(--encre);
    letter-spacing: -0.01em;
  }

  .plan {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--encre-douce);
    border: 1px solid var(--trace);
    border-radius: 4px;
    padding: 1px 5px;
  }

  .rafraichir {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    padding: 0;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--encre-douce);
    cursor: pointer;
    transition: background 0.15s, color 0.15s;
  }

  .rafraichir svg {
    width: 14px;
    height: 14px;
  }

  .rafraichir:hover:not(:disabled) {
    background: var(--trace);
    color: var(--encre);
  }

  .rafraichir:disabled {
    cursor: progress;
  }

  .tourne svg {
    animation: tourner 0.9s linear infinite;
  }

  @keyframes tourner {
    to {
      transform: rotate(360deg);
    }
  }

  /* Respecte les reglages d'accessibilite : pas d'animation imposee. */
  @media (prefers-reduced-motion: reduce) {
    .tourne svg {
      animation: none;
    }
  }

  .jauges {
    flex: 1;
    display: flex;
    flex-direction: column;
    justify-content: center;
    padding: 4px 0;
  }

  .vide {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    text-align: center;
    color: var(--encre-douce);
  }

  .vide p {
    margin: 0;
    font-size: 12px;
  }

  .pourquoi {
    font-size: 11px;
    opacity: 0.75;
    line-height: 1.45;
  }

  .sonder {
    margin-top: 4px;
    padding: 5px 12px;
    font-size: 11px;
    font-weight: 600;
    color: var(--encre);
    background: transparent;
    border: 1px solid var(--bord);
    border-radius: 6px;
    cursor: pointer;
  }

  .sonder:hover:not(:disabled) {
    background: var(--trace);
  }

  footer {
    display: flex;
    align-items: center;
    gap: 6px;
    padding-top: 8px;
    border-top: 1px solid var(--trace);
    font-size: 10px;
    color: var(--encre-douce);
    font-variant-numeric: tabular-nums;
  }

  .point {
    width: 5px;
    height: 5px;
    border-radius: 50%;
  }

  .vert-point {
    background: var(--vert);
  }

  .note {
    margin-left: auto;
    display: grid;
    place-items: center;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--ambre);
    color: #fff;
    font-size: 9px;
    font-weight: 700;
    cursor: help;
  }
</style>
