import agency from "../../assets/map_splash/agency.jpg";
import ancient from "../../assets/map_splash/ancient.jpg";
import anubis from "../../assets/map_splash/anubis.jpg";
import baggage from "../../assets/map_splash/baggage.jpg";
import basalt from "../../assets/map_splash/basalt.jpg";
import dust2 from "../../assets/map_splash/dust2.png";
import edin from "../../assets/map_splash/edin.jpg";
import grail from "../../assets/map_splash/grail.jpg";
import inferno from "../../assets/map_splash/inferno.png";
import italy from "../../assets/map_splash/italy.jpg";
import jura from "../../assets/map_splash/jura.jpg";
import mirage from "../../assets/map_splash/mirage.jpg";
import nuke from "../../assets/map_splash/nuke.jpg";
import office from "../../assets/map_splash/office.jpg";
import overpass from "../../assets/map_splash/overpass.jpg";
import palais from "../../assets/map_splash/palais.jpg";
import poolDay from "../../assets/map_splash/pool_day.jpg";
import shoots from "../../assets/map_splash/shoots.jpg";
import train from "../../assets/map_splash/train.jpg";
import vertigo from "../../assets/map_splash/vertigo.jpg";
import whistle from "../../assets/map_splash/whistle.jpg";

const artwork: Record<string, string> = {
  agency, ancient, anubis, baggage, basalt, dust2, edin, grail, inferno, italy, jura,
  mirage, nuke, office, overpass, palais, pool_day: poolDay, shoots, train, vertigo, whistle,
};

export function artworkForMap(map: string | null): string | null {
  const key = map?.toLowerCase().replace(/^de_|^cs_|^ar_/, "");
  return key ? artwork[key] ?? null : null;
}
