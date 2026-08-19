// Marks the functions worth looking at first.
//
// A stripped binary gives you a thousand addresses and no order to read them
// in. Size is a poor proxy for importance and a very good proxy for "this one
// took someone a while to write", which is where a reader usually starts.

const LARGE = 512;

let marked = 0;

for f in functions() {
    if f.size < LARGE {
        continue;
    }

    // How many places name this address: a large function nothing calls is a
    // different kind of interesting from a large function called everywhere.
    let callers = ref_count(f.address);

    comment(f.address, `${f.size} bytes, ${callers} references`);
    bookmark(f.address);
    marked += 1;
}

print(`marked ${marked} functions of ${LARGE} bytes or more`);
