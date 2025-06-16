const timeElements = document.getElementsByClassName("time");

for (const timeEl of timeElements) {
    const isLongTime = timeEl.classList.contains("long-time");
    const enableDiscordCopy = timeEl.classList.contains("discord");
    var format;
    if (isLongTime) {
        format = {weekday: 'long', year: 'numeric', month: 'long', day: 'numeric', hour: 'numeric', minute: '2-digit', timeZoneName: 'longGeneric'}
    }

    if (!format) {
        console.log("Time element missing proper time format, ignoring");
        continue;
    }

    const time = timeEl.innerText + "Z";
    const parsedTime = Date.parse(time);
    timeEl.innerText = new Intl.DateTimeFormat('default',
        format
    ).format(parsedTime);

    if (enableDiscordCopy) {
        const copyEl = document.createElement("i");
        copyEl.title = "Copy discord timestamp";
        copyEl.onclick = () => {
            navigator.clipboard.writeText("<t:" + parsedTime / 1000 + ":F>").await;
            copyEl.classList = "fa fa-check copy-button"
            setTimeout(() => copyEl.classList = "fa-regular fa-copy copy-button", 500)
            return true;
        }
        copyEl.classList = "fa-regular fa-copy copy-button";
        timeEl.insertAdjacentElement('beforeend', copyEl);
    }
}
