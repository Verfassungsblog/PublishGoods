/**
 * Mutual exclude for JavaScript.
 *
 * @module mutex
 */

/**
 * Creates a mutual exclude function with the following property:
 *
 * ```js
 * const mutex = createMutex()
 * mutex(() => {
 *   // This function is immediately executed
 *   mutex(() => {
 *     // This function is not executed, as the mutex is already active.
 *   })
 * })
 * ```
 *
 * @return {function(Function, Function=):void} A mutual exclude function
 * @public
 */
export const createMutex = () => {
    let token = true
    return (f: () => void, g?: () => void) => {
        if (token) {
            token = false
            try {
                f()
            } finally {
                token = true
            }
        } else if (g !== undefined) {
            g()
        }
    }
}
