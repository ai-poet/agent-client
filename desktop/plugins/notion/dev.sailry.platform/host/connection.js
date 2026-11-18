import {card} from "sailry/cards";
import {status} from "sailry/connectors";
import {fallback} from "../desktop/locales.js";

export async function connection() {
  const connection = await status("notion");
  const configured = connection?.enabled === true && connection.state === "connected";
  const result = {configured, sailry_content: {version: 1, blocks: [
    card("connection", {configured}, fallback(configured ? "connected" : "connect")),
  ]}};
  return result;
}
