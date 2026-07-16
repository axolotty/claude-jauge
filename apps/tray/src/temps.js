/**
 * Mise en forme des durees et des heures.
 *
 * Cote Rust, `human_duration` fait la meme chose pour le CLI. Ici on la refait
 * en JS plutot que de la faire traverser le pont : le compte a rebours avance
 * chaque seconde, et il serait absurde de reveiller Rust pour ca.
 */

/** Duree compacte : `3j02h`, `1h49`, `12m`, `45s`. */
export function duree(secondes) {
  if (secondes == null) return null;
  if (secondes <= 0) return "maintenant";

  const minutes = Math.floor(secondes / 60);
  const heures = Math.floor(minutes / 60);
  const jours = Math.floor(heures / 24);

  if (jours > 0) return `${jours}j${String(heures % 24).padStart(2, "0")}h`;
  if (heures > 0) return `${heures}h${String(minutes % 60).padStart(2, "0")}`;
  if (minutes > 0) return `${minutes}m`;
  // Sous la minute, les secondes defilent : c'est le moment ou ca compte.
  return `${secondes}s`;
}

/** Heure locale. Le jour n'apparait que si ce n'est pas aujourd'hui. */
export function heure(epoch) {
  if (epoch == null) return null;
  const date = new Date(epoch * 1000);
  const aujourdhui = new Date().toDateString() === date.toDateString();

  return aujourdhui
    ? date.toLocaleTimeString("fr-FR", { hour: "2-digit", minute: "2-digit" })
    : date.toLocaleString("fr-FR", {
        weekday: "short",
        day: "2-digit",
        month: "2-digit",
        hour: "2-digit",
        minute: "2-digit",
      });
}
