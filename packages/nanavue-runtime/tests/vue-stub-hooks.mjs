export async function resolve(specifier, context, nextResolve) {
  if (specifier === "@vue/runtime-core") {
    return {
      shortCircuit: true,
      url: context.data?.stub || new URL("./vue-runtime-core-stub.mjs", import.meta.url).href,
    };
  }
  return nextResolve(specifier, context);
}
