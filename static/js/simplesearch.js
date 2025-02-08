class SearchItem {
    constructor(value, element) {
        this.value = value;
        this.element = element;
    }

    hide() {
        this.element.style.display = 'none';
    }

    show() {
        this.element.style.display = 'table-row';
    }
}


class SimpleSearch {
    constructor(input, itemSelector, valueSelector) {
        this.input = document.getElementsByName(input)[0] || document.getElementById(input);
        this.items = [];
        for (var element of Array.prototype.slice.call(document.querySelectorAll(itemSelector))) {
            const valueElmt = element.querySelectorAll(valueSelector)[0];
            this.items.push(new SearchItem(
                valueElmt.innerText + " " + valueElmt.title,
                element
            ));
        }

        if (!this.input) {
            console.log("simplesearch.js::error::Could not find input with id " + input);
        }

        this.input.addEventListener('keyup', () => {
            this.update();
        });
    }

    update() {
        var reg = new RegExp(this.input.value, 'i');
        for (var item of this.items) {
            if (reg.test(item.value) && item.element.dataset.hidden !== "true") {
                item.show();
            } else {
                item.hide();
            }
        }
    }
}
