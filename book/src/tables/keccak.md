## Keccak-f

This table computes the Keccak-f\[1600\] permutation.

### Keccak-f Permutation

To explain how this table is structured, we first need to detail how the
permutation is computed. [This
page](https://keccak.team/keccak_specs_summary.html) gives a pseudo-code
for the permutation. Our implementation differs slightly -- but remains
equivalent -- for optimization and constraint degree reasons.

Let:

-   $S$ be the sponge width ($S=25$ in our case)

-   $\texttt{NUM\_ROUNDS}$ be the number of Keccak rounds
    ($\texttt{NUM\_ROUNDS} = 24$)

-   $RC$ a vector of round constants of size $\texttt{NUM\_ROUNDS}$

-   $I$ be the input of the permutation, comprised of $S$ 64-bit
    elements

The first step is to reshape $I$ into a $5 \times 5$ matrix. We
initialize the state $A$ of the sponge with $I$:
$$A[x, y] := I[x, y] \text{ }  \forall x, y \in \{0..4\}$$

We store $A$ in the table, and subdivide each 64-bit element into two
32-bit limbs. Then, for each round $i$, we proceed as follows:

1.  First, we define $C[x] := \texttt{xor}_{i=0}^4 A[x, i]$. We store
    $C$ as bits in the table. This is because we need to apply a
    rotation on its elements' bits and carry out ` xor ` operations in
    the next step.

2.  Then, we store a second vector $C'$ in bits, such that:
    $$C'[x, z] = C[x, z] \texttt{ xor } C[x-1, z] \texttt{ xor } C[x+1, z-1]$$.

3.  We then need to store the updated value of $A$:
    $$A'[x, y] = A[x, y] \texttt{ xor } C[x, y] \texttt{ xor } C'[x, y]$$
    Note that this is equivalent to the equation in the official
    Keccak-f description:
    $$A'[x, y] = A[x, y] \texttt{ xor } C[x-1, z] \texttt{ xor } C[x+1, z-1]$$.

4.  The previous three points correspond to the $\theta$ step in
    Keccak-f. We can now move on to the $\rho$ and $\pi$ steps. These
    steps are written as:
    $$B[y, 2\times x + 3 \times y] := \texttt{rot}(A'[x, y], r[x, y])$$
    where $\texttt{rot(a, s)}$ is the bitwise cyclic shift operation,
    and $r$ is the matrix of rotation offsets. We do not need to store
    $B$: $B$'s bits are only a permutation of $A'$'s bits.

5.  The $\chi$ step updates the state once again, and we store the new
    values:
    $$A''[x, y] := B[x, y] \texttt{ xor } (\texttt{not }B[x+1, y] \texttt{ and } B[x+2, y])$$
    Because of the way we carry out constraints (as explained below), we
    do not need to store the individual bits for $A''$: we only need the
    32-bit limbs.

6.  The final step, $\iota$, consists in updating the first element of
    the state as follows:
    $$A'''[0, 0] = A''[0, 0] \texttt{ xor } RC[i]$$ where
    $$A'''[x, y] = A''[x, y] \forall (x, y) \neq (0, 0)$$ Since only the
    first element is updated, we only need to store $A'''[0, 0]$ of this
    updated state. The remaining elements are fetched from $A''$.
    However, because of the bitwise $\texttt{xor}$ operation, we do need
    columns for the bits of $A''[0, 0]$.

Note that all permutation elements are 64-bit long. But they are stored
as 32-bit limbs so that we do not overflow the field.

It is also important to note that all bitwise logic operations
($\texttt{ xor }$, $\texttt{ not }$ and $\texttt{ and}$) are checked in
this table. This is why we need to store the bits of most elements. The
logic table can only carry out eight 32-bit logic operations per row.
Thus, leveraging it here would drastically increase the number of logic
rows, and incur too much overhead in proving time.

### Columns

Using the notations from the previous section, we can now list the
columns in the table:

1.  $\texttt{NUM\_ROUND}S = 24$ columns $c_i$ to determine which round
    is currently being computed. $c_i = 1$ when we are in the $i$-th
    round, and 0 otherwise. These columns' purpose is to ensure that the
    correct round constants are used at each round.

2.  $1$ column $t$ which stores the timestamp at which the Keccak
    operation was called in the cpu. This column enables us to ensure
    that inputs and outputs are consistent between the cpu,
    keccak-sponge and keccak-f tables.

3.  $5 \times 5 \times 2 = 50$columns to store the elements of $A$. As a
    reminder, each 64-bit element is divided into two 32-bit limbs, and
    $A$ comprises $S = 25$ elements.

4.  $5 \times 64 = 320$ columns to store the bits of the vector $C$.

5.  $5 \times 64 = 320$ columns to store the bits of the vector $C'$.

6.  $5 \times 5 \times 64 = 1600$ columns to store the bits of $A'$.

7.  $5 \times 5 \times 2 = 50$ columns to store the 32-bit limbs of
    $A''$.

8.  $64$ columns to store the bits of $A''[0, 0]$.

9.  $2$ columns to store the two limbs of $A'''[0, 0]$.

In total, this table comprises 2,431 columns.

### Constraints

Some constraints checking that the elements are computed correctly are
not straightforward. Let us detail them here.

First, it is important to highlight the fact that a $\texttt{xor}$
between two elements is of degree 2. Indeed, for $x \texttt{ xor } y$,
the constraint is $x + y - 2 \times x \times y$, which is of degree 2.
This implies that a $\texttt{xor}$ between 3 elements is of degree 3,
which is the maximal constraint degree for our STARKs.

We can check that
$C'[x, z] = C[x, z] \texttt{ xor } C[x - 1, z] \texttt{ xor } C[x + 1, z - 1]$.
However, we cannot directly check that
$C[x] = \texttt{xor}_{i=0}^4 A[x, i]$, as it would be a degree 5
constraint. Instead, we use $C'$ for this constraint. We see that:
$$\texttt{xor}_{i=0}^4 A'[x, i, z] = C'[x, z]$$ This implies that the
difference $d = \sum_{i=0}^4 A'[x, i, z] - C'[x, z]$ is either 0, 2 or
4. We can therefore enforce the following degree 3 constraint instead:
$$d \times (d - 2) \times (d - 4) = 0$$

Additionally, we have to check that $A'$ is well constructed. We know
that $A'$ should be such that
$A'[x, y, z] = A[x, y, z] \texttt{ xor } C[x, z] \texttt{ xor } C'[x, z]$.
Since we do not have the bits of $A$ elements but the bits of $A'$
elements, we check the equivalent degree 3 constraint:
$$A[x, y, z] = A'[x, y, z] \texttt{ xor } C[x, z] \texttt { xor } C'[x, z]$$

Finally, the constraints for the remaining elements, $A''$ and $A'''$
are straightforward: $A''$ is a three-element bitwise $\texttt{xor}$
where all bits involved are already storedn and $A'''[0, 0]$ is the
output of a simple bitwise $\texttt{xor}$ with a round constant.
