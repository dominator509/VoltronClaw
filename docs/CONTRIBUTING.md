# Contributing

## Developer Certificate of Origin (DCO)

All contributors must sign off on their commits using `git commit -s`.
This certifies:

```
Developer Certificate of Origin
Version 1.1

By making a contribution to this project, I certify that:

(a) The contribution was created in whole or in part by me and I
    have the right to submit it under the open source license
    indicated in the file; or

(b) The contribution is based upon previous work that, to the best
    of my knowledge, is covered under an appropriate open source
    license and I have the right under that license to submit that
    work with modifications, whether created in whole or in part
    by me, under the same open source license (unless I am
    permitted to submit under a different license), as indicated
    in the file; or

(c) The contribution was provided directly to me by some other
    person who certified (a), (b) or (c) and I have not modified
    it.

(d) I understand and agree that this project and the contribution
    are public and that a record of the contribution (including all
    personal information I submit with it, including my sign-off) is
    maintained indefinitely and may be redistributed consistent with
    this project or the open source license(s) involved.
```

## Third-Party Code

If your contribution introduces third-party code, you MUST:

1. Complete the pre-intake audit checklist (LICENSE_STRATEGY.md §3)
2. Attach the completed checklist to your PR
3. Ensure the code is placed in `/third_party/<component>/` with its
   original LICENSE
4. Update `THIRD_PARTY_LICENSES.md` and `IP_POSTURE_MATRIX.md`

PRs that lack a completed checklist will not be merged.

## License

By contributing, you agree that your contributions are licensed under
the Apache-2.0 license (see LICENSE in the repo root).
