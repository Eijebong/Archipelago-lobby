const spanElements = document.getElementsByTagName("span");

for (const span of spanElements) {
    const hasTitle = span.title !== undefined && span.title !== "";

    if (!hasTitle) {
        continue;
    }

    const parent = span.parentElement;
    const titleElem = document.createElement("span");
    titleElem.innerHTML = " (" + span.title + ")";
    titleElem.style.display = 'none';
  titleElem.classList = "span-title"
    if (span.nextSibling) {
        parent.insertBefore(titleElem, span.nextSibling);
    } else {
        parent.appendChild(titleElem);
    }

    parent.addEventListener('mouseover', () => {
        titleElem.style.display = 'inline';
    });
    parent.addEventListener('mouseleave', () => {
        titleElem.style.display = 'none';
    });

}

