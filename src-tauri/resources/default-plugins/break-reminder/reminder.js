(() => {
const pluginApi = window.ztools;
const settingsView = document.querySelector('#settings-view');
const breakView = document.querySelector('#break-view');
const enabledInput = document.querySelector('#enabled');
const intervalInput = document.querySelector('#interval');
const durationInput = document.querySelector('#duration');
const saveStatus = document.querySelector('#save-status');
const countdown = document.querySelector('#countdown');
let countdownTimer = null;

/**
 * 将输入值限制到可用的整数范围，避免无效设置进入后台计时器。
 * @param {unknown} value 用户输入或存储值。
 * @param {number} minimum 最小允许值。
 * @param {number} maximum 最大允许值。
 * @param {number} fallback 无效时使用的默认值。
 * @returns {number} 合法整数。
 */
function boundedInteger(value, minimum, maximum, fallback) {
  const number = Number(value);
  return Number.isInteger(number) && number >= minimum && number <= maximum ? number : fallback;
}

/**
 * 从插件私有存储读取设置，并为旧数据补齐默认值。
 * @returns {{enabled: boolean, intervalMinutes: number, durationSeconds: number}} 当前设置。
 */
function readSettings() {
  const saved = pluginApi.dbStorage.getItem('settings') || {};
  return {
    enabled: saved.enabled === true,
    intervalMinutes: boundedInteger(saved.intervalMinutes, 1, 480, 20),
    durationSeconds: boundedInteger(saved.durationSeconds, 1, 600, 10)
  };
}

/**
 * 把当前设置呈现到表单，重新进入插件时与后台存储保持一致。
 * @returns {void} 无返回值。
 */
function renderSettings() {
  const settings = readSettings();
  enabledInput.checked = settings.enabled;
  intervalInput.value = String(settings.intervalMinutes);
  durationInput.value = String(settings.durationSeconds);
  saveStatus.textContent = settings.enabled ? '提醒已启用' : '提醒已关闭';
}

/**
 * 校验并持久化表单，后台计时器下一次轮询会应用新配置。
 * @returns {void} 无返回值。
 */
function saveSettings() {
  const settings = {
    enabled: enabledInput.checked,
    intervalMinutes: boundedInteger(intervalInput.value, 1, 480, 20),
    durationSeconds: boundedInteger(durationInput.value, 1, 600, 10)
  };
  // 将被纠正的输入同步回界面，避免显示值与实际计时不一致。
  intervalInput.value = String(settings.intervalMinutes);
  durationInput.value = String(settings.durationSeconds);
  pluginApi.dbStorage.setItem('settings', settings);
  saveStatus.textContent = settings.enabled ? '已保存，正在计时' : '已保存，提醒已关闭';
}

/**
 * 关闭当前休息弹窗，不影响下一轮已排好的提醒。
 * @returns {Promise<void>} 窗口隐藏并退出后结束的 Promise。
 */
async function closeBreak() {
  if (countdownTimer !== null) clearInterval(countdownTimer);
  countdownTimer = null;
  // 宿主将本次提醒标记为不恢复启动器，关闭后原来的前台应用保持不变。
  await pluginApi.outPlugin();
}

/**
 * 显示一次休息提醒并在持续时间结束后自动关闭。
 * @param {{payload?: {durationSeconds?: number}}} action 宿主派发的提醒参数。
 * @returns {Promise<void>} 弹窗布局完成后结束的 Promise。
 */
async function showBreak(action) {
  if (countdownTimer !== null) clearInterval(countdownTimer);
  settingsView.hidden = true;
  breakView.hidden = false;
  const seconds = boundedInteger(action?.payload?.durationSeconds, 1, 600, readSettings().durationSeconds);
  const endsAt = Date.now() + seconds * 1000;
  // 用绝对时间计算剩余秒数，减少后台或系统休眠带来的倒计时漂移。
  /**
   * 刷新剩余时间，并在本次休息完成时关闭弹窗。
   * @returns {void} 无返回值。
   */
  const updateCountdown = () => {
    const remaining = Math.max(0, Math.ceil((endsAt - Date.now()) / 1000));
    countdown.textContent = `${remaining} 秒后结束`;
    if (remaining === 0) void closeBreak();
  };
  updateCountdown();
  countdownTimer = setInterval(updateCountdown, 250);
  await pluginApi.window.setSize(440, 360);
  await pluginApi.window.center();
}

/**
 * 根据启动器动作切换设置页或本次休息提醒。
 * @param {{code?: string, payload?: object}} action 插件进入动作。
 * @returns {Promise<void>} 对应页面准备完成后结束的 Promise。
 */
async function onEnter(action) {
  if (action?.code === 'break') {
    await showBreak(action);
    return;
  }
  if (countdownTimer !== null) clearInterval(countdownTimer);
  countdownTimer = null;
  breakView.hidden = true;
  settingsView.hidden = false;
  renderSettings();
  await pluginApi.window.setSize(540, 530);
  await pluginApi.window.center();
}

/**
 * 等待设置写入完成后关闭设置页，并返回启动器。
 * @returns {Promise<void>} 插件窗口关闭后结束的 Promise。
 */
async function closeSettings() {
  await pluginApi.outPlugin();
}

enabledInput.addEventListener('change', saveSettings);
intervalInput.addEventListener('change', saveSettings);
durationInput.addEventListener('change', saveSettings);
document.querySelector('#close-settings').addEventListener('click', closeSettings);
document.querySelector('#skip-break').addEventListener('click', closeBreak);
pluginApi.onPluginEnter(onEnter);
})();
