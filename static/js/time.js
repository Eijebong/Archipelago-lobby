const timeElements = document.getElementsByClassName("time");

for (const timeEl of timeElements) {
    const isLongTime = timeEl.classList.contains("long-time");
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
}
