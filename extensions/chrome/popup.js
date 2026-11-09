const status = document.getElementById("status");
const detail = document.getElementById("detail");
const showButton = document.getElementById("show");
const copyButton = document.getElementById("copy");

async function copyError() {
  await navigator.clipboard.writeText(detail.textContent);
  copyButton.textContent = "已复制";
}

function failed(error) {
  status.textContent = "保存失败";
  detail.textContent = error || "未知错误";
  copyButton.hidden = false;
  copyButton.onclick = copyError;
  detail.onclick = copyError;
}

chrome.runtime.sendMessage({ type: "capture" }, (reply) => {
  if (!reply || !reply.ok) return failed(reply?.error);
  status.textContent = "已保存到 Eddy";
  showButton.hidden = false;
  showButton.focus();
  showButton.onclick = () => {
    showButton.disabled = true;
    chrome.runtime.sendMessage({ type: "show", url: reply.url }, (shown) => {
      if (shown?.ok) {
        window.close();
      } else {
        showButton.hidden = true;
        failed(shown?.error);
      }
    });
  };
});
