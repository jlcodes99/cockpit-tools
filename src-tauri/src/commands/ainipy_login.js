(() => {
  if (location.origin !== 'https://www.ainipy.com' || window.top !== window) return;
  const timer = setInterval(() => {
    const credential = sessionStorage.getItem('ainipy.admin_credential');
    if (!credential) return;
    clearInterval(timer);
    // Native navigation interception cancels this request before it reaches the server.
    location.href = 'https://www.ainipy.com__AINIPY_CALLBACK_PATH__#token=' + encodeURIComponent(credential);
  }, 500);
})();
