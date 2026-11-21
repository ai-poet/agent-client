const en = {
  review_files: files => `${files} ${files === 1 ? 'file' : 'files'}`,
};
const zh = {
  review_files: files => `${files} 个文件`,
};

export function completion(files) {
  return {label:en.review_files(files),locales:{'zh-CN':zh.review_files(files)}};
}
