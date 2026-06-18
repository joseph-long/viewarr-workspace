import {
  JupyterFrontEnd,
  JupyterFrontEndPlugin
} from '@jupyterlab/application';
import {
  ABCWidgetFactory,
  DocumentModel,
  DocumentRegistry
} from '@jupyterlab/docregistry';
import {
  Contents,
  IDefaultDrive,
  RestContentProvider
} from '@jupyterlab/services';
import { FITSDocument, FITSPanel } from './widget';

/**
 * The MIME type for FITS files
 */
const MIME_TYPE = 'image/fits';

/**
 * The name of the factory
 */
const FACTORY = 'FITS Viewer';

/**
 * Content provider ID for FITS files
 */
const CONTENT_PROVIDER_ID = 'fitsview-provider';

/**
 * A content provider that fetches file metadata without content.
 * This prevents downloading the entire FITS file to the browser.
 */
class FITSContentProvider extends RestContentProvider {
  async get(
    localPath: string,
    options?: Contents.IFetchOptions
  ): Promise<Contents.IModel> {
    // Always fetch without content - we'll use our API for data access
    return super.get(localPath, { ...options, content: false });
  }
}

/**
 * A widget factory for FITS files.
 */
class FITSViewerFactory extends ABCWidgetFactory<FITSDocument, DocumentModel> {
  protected createNewWidget(
    context: DocumentRegistry.IContext<DocumentModel>
  ): FITSDocument {
    const content = new FITSPanel(context);
    return new FITSDocument({ context, content });
  }
}

/**
 * Model factory for FITS documents
 */
class FITSModelFactory implements DocumentRegistry.IModelFactory<DocumentModel> {
  /**
   * The name of the model.
   */
  get name(): string {
    return 'fits-model';
  }

  /**
   * The content type of the file.
   */
  get contentType(): Contents.ContentType {
    return 'file';
  }

  /**
   * The format of the file.
   */
  get fileFormat(): Contents.FileFormat {
    // Return null to indicate we don't want content loaded
    return null as any;
  }

  /**
   * Whether the model is disposed.
   */
  get isDisposed(): boolean {
    return this._disposed;
  }

  /**
   * Dispose of the model factory.
   */
  dispose(): void {
    this._disposed = true;
  }

  /**
   * Get the preferred kernel language given a file path.
   */
  preferredLanguage(path: string): string {
    return '';
  }

  /**
   * Create a new model for a given path.
   */
  createNew(options?: DocumentRegistry.IModelOptions<any>): DocumentModel {
    return new DocumentModel();
  }

  private _disposed = false;
}

/**
 * The FITS viewer plugin.
 */
const plugin: JupyterFrontEndPlugin<void> = {
  id: 'fitsview:plugin',
  description: 'View FITS files in JupyterLab without downloading full content',
  autoStart: true,
  requires: [IDefaultDrive],
  activate: (app: JupyterFrontEnd, defaultDrive: Contents.IDrive) => {
    console.log('JupyterLab extension fitsview is activated!');

    // Register the content provider that skips content loading
    const registry = defaultDrive.contentProviderRegistry;
    if (registry) {
      const serverSettings = (defaultDrive as any).serverSettings;
      registry.register(
        CONTENT_PROVIDER_ID,
        new FITSContentProvider({
          apiEndpoint: 'api/contents',
          serverSettings
        })
      );
      console.log('Registered FITS content provider');
    }

    // Register the file type
    app.docRegistry.addFileType({
      name: 'fits',
      displayName: 'FITS Image',
      mimeTypes: [MIME_TYPE],
      extensions: ['.fits', '.fit', '.fts'],
      fileFormat: 'base64',
      contentType: 'file',
      icon: undefined // TODO: Add a FITS icon
    });

    // Register the model factory
    const modelFactory = new FITSModelFactory();
    app.docRegistry.addModelFactory(modelFactory);

    // Register the widget factory
    const widgetFactory = new FITSViewerFactory({
      name: FACTORY,
      modelName: 'fits-model',
      fileTypes: ['fits'],
      defaultFor: ['fits'],
      contentProviderId: CONTENT_PROVIDER_ID
    });
    app.docRegistry.addWidgetFactory(widgetFactory);

    console.log('FITS viewer factory registered');
  }
};

export default plugin;
