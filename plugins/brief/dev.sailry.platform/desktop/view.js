import {div} from 'gpui-kit';
import {Header} from 'sailry';
import {IconButton} from 'sailry/ui';
import {ArtifactSurface} from 'sailry/artifacts';

export function render(view) {
  const {state,text} = view,unavailable = !state.connected;
  const exportDisabled = unavailable || state.exporting || !state.artifact;
  const header = Header.new('brief-header',{content:JSON.stringify({title:state.artifact?.title ?? '',
    filters:[{id:'brief-version',label:text.brief_version,selected:state.revision === null ? '' : String(state.revision),
      primary:true,disabled:unavailable || !state.artifact,items:state.versions()}],
    actions:[...(state.before === null ? [] : [{id:'brief-more-versions',icon:'icons/ellipsis.svg',label:text.brief_more_versions,disabled:unavailable}]),
      {id:'brief-export',label:text.brief_export,icon:'icons/ellipsis-vertical.svg',disabled:exportDisabled,
        menu:[{id:'brief-export-html',label:text.brief_html,enabled:!exportDisabled},
          ...(state.artifact?.kind === 'document' ? [{id:'brief-export-image',label:text.brief_image,enabled:!exportDisabled}] : [])]}],
  })});
  const content = state.artifact ? ArtifactSurface.new(`brief-preview-${state.artifact.id}`,{
    artifact:state.artifact.id,...(state.follow && !state.loading && state.revision === state.artifact.revision
      ? {} : {revision:state.follow ? state.artifact.revision : state.revision}),
    labels:{contents:text.brief_contents,submit:text.brief_submit,required:text.brief_form_required,
      invalid_form:text.brief_form_invalid},
  }) : state.loading ? div().size_full()
    : div().size_full().h_flex().justify_center().items_center().child(IconButton.new('brief-retry',{
      icon:'rotate-cw',label:text.brief_retry,disabled:!state.resource || !state.connected,show_label:true,
    }));
  return div().id('brief-panel').v_flex().size_full().min_w_0().min_h_0().child(header)
    .child(div().flex_1().min_w_0().min_h_0().child(content));
}
