# Pack format reference (sketch)

A proposal for discussion, not a finished page.

## Record

    {
      "id": "mem_01H...",
      "kind": "fact" | "preference" | "procedure",
      "body": "...",
      "score": 0.0 - 1.0,
      "prev": "<hash of the previous record>",
      "hash": "<hash of this record>"
    }

## Open questions

- Should a pack carry its own manifest, or is the chain enough to walk it?
- Do we version the record layout, and how do old packs migrate?
- What is the maximum sensible pack size before we want sharding?
