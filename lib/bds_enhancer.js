/**
 * @typedef {import("@minecraft/server").Player} Player
 */
import { system } from "@minecraft/server";
/**
 * @param {string} action
 * @param {unknown} payload
 */
function sendAction(action, payload) {
  const json = JSON.stringify({ action, payload });
  console.warn(`bds_enhancer:${json}`);
}

/**
 * @param {Player} player
 * @param {string} host
 * @param {number} port
 */
export function sendTransferAction(player, host, port) {
  sendAction("transfer", { player: player.name, host, port });
}

/**
 * @param {Player} player
 * @param {string} reason
 */
export function sendKickAction(player, reason) {
  sendAction("kick", { player: player.name, reason });
}

export function sendStopAction() {
  sendAction("stop");
}

export function sendReloadAction() {
  sendAction("reload");
}

/**
 * @type {{command: string, resolve: (value: string) => void}[]}
 */
const commandQueue = [];

/**
 * @template {boolean} T
 * @param {string} command 
 * @param {T} result 
 * @returns {Promise<T extends true ? string : undefined>}
 */
export async function executeCommand(command = "", result = false) {
  sendAction("execute", { command, result });
  return new Promise((resolve) => {
    if (result) commandQueue.push({ command, resolve });
    else resolve();
  });
}

system.afterEvents.scriptEventReceive.subscribe((event) => {
  if (event.id === "bds_enhancer:result") {
    /**
     * @type {{command: string, result_message: string}}
     */
    const data = JSON.parse(event.message);
    commandQueue.find((item) => item.command === data.command)?.resolve(data.result_message);
  }
});
