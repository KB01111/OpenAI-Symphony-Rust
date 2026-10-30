/** Shell scripts are imported as their text (the bundler's `text` loader; see scripts/build.mjs). */
declare module "*.sh" {
  const content: string;
  export default content;
}
