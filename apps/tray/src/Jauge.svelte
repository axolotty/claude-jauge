<script>
  import { duree, heure } from "./temps.js";

  /** @type {{ fenetre: any, maintenant: number }} */
  let { fenetre, maintenant } = $props();

  const RAYON = 16;
  const CIRCONFERENCE = 2 * Math.PI * RAYON;

  let taux = $derived(fenetre.used ?? 0);
  let inconnu = $derived(fenetre.used == null);

  // Meme seuils que l'icone et le CLI : il n'y a qu'une seule verite visuelle.
  let ton = $derived(
    inconnu ? "inconnu" : taux >= 80 ? "rouge" : taux >= 50 ? "ambre" : "vert",
  );

  let rempli = $derived((Math.min(taux, 100) / 100) * CIRCONFERENCE);
  let restant = $derived(fenetre.reset_at != null ? fenetre.reset_at - maintenant : null);
</script>

<div class="jauge" class:sourd={inconnu}>
  <svg viewBox="0 0 40 40" aria-hidden="true">
    <!-- L'anneau de fond montre ce qui reste. -->
    <circle class="fond" cx="20" cy="20" r={RAYON} />
    <circle
      class="arc {ton}"
      cx="20"
      cy="20"
      r={RAYON}
      stroke-dasharray="{rempli} {CIRCONFERENCE}"
    />
  </svg>

  <div class="texte">
    <div class="entete">
      <span class="titre">{fenetre.label}</span>
      <span class="taux {ton}">{inconnu ? "?" : Math.round(taux) + " %"}</span>
    </div>

    <div class="detail">
      {#if fenetre.health === "blocked"}
        <span class="alerte rouge">epuise</span>
      {:else if fenetre.health === "warning"}
        <span class="alerte ambre">limite proche</span>
      {:else if !inconnu}
        <span>reste {Math.round(fenetre.remaining)} %</span>
      {/if}

      {#if restant != null && restant > 0}
        <span class="separateur">·</span>
        <span title="reset a {heure(fenetre.reset_at)}">{duree(restant)}</span>
      {:else if !inconnu}
        <span class="separateur">·</span>
        <span class="sourdine">fenetre non ouverte</span>
      {/if}
    </div>
  </div>
</div>

<style>
  .jauge {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 4px;
  }

  svg {
    width: 40px;
    height: 40px;
    flex-shrink: 0;
    /* Fait partir l'arc de midi plutot que de 3 heures. */
    transform: rotate(-90deg);
  }

  circle {
    fill: none;
    stroke-width: 5;
    stroke-linecap: round;
  }

  .fond {
    stroke: var(--trace);
  }

  .arc {
    transition: stroke-dasharray 0.5s ease-out, stroke 0.3s;
  }

  .texte {
    flex: 1;
    min-width: 0;
  }

  .entete {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 8px;
  }

  .titre {
    font-size: 12px;
    font-weight: 600;
    color: var(--encre);
  }

  .taux {
    font-size: 13px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }

  .detail {
    display: flex;
    align-items: baseline;
    gap: 5px;
    margin-top: 2px;
    font-size: 11px;
    color: var(--encre-douce);
    font-variant-numeric: tabular-nums;
  }

  .separateur {
    opacity: 0.4;
  }

  .sourdine {
    opacity: 0.6;
  }

  .alerte {
    font-weight: 600;
  }

  .sourd {
    opacity: 0.55;
  }

  .vert {
    stroke: var(--vert);
    color: var(--vert);
  }
  .ambre {
    stroke: var(--ambre);
    color: var(--ambre);
  }
  .rouge {
    stroke: var(--rouge);
    color: var(--rouge);
  }
  .inconnu {
    stroke: var(--gris);
    color: var(--gris);
  }
</style>
